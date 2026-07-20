#include "dusk/automation/card_fixture.hpp"

#include <algorithm>
#include <array>
#include <fstream>
#include <limits>
#include <system_error>
#include <utility>
#include <vector>

#include <xxhash.h>

namespace dusk::automation {
namespace {

constexpr std::string_view IdentityDomain = "dusklight-automation-card-fixture/v1";
std::string sActiveIdentity;

struct FixtureFile {
    std::filesystem::path source;
    std::filesystem::path relative;
    std::string key;
    std::uint64_t size = 0;
};

bool path_is_within(const std::filesystem::path& child, const std::filesystem::path& parent) {
    const std::filesystem::path relative = child.lexically_relative(parent);
    if (relative.empty())
        return child == parent;
    const auto first = relative.begin();
    return first != relative.end() && *first != "..";
}

bool valid_fixture_path(const std::filesystem::path& relative) {
    if (relative.empty() || relative.is_absolute())
        return false;
    std::vector<std::filesystem::path> components;
    for (const auto& component : relative)
        components.push_back(component);
    if (components.size() != 3 || components[0] == "." || components[0] == ".." ||
        components[1] == "." || components[1] == ".." || components[2] == "." ||
        components[2] == "..")
        return false;
    const std::string region = components[0].string();
    const std::string card = components[1].string();
    const std::string extension = components[2].extension().string();
    return (region == "USA" || region == "EUR" || region == "JAP") &&
           (card == "Card A" || card == "Card B") && extension == ".gci";
}

bool valid_fixture_directory(const std::filesystem::path& relative) {
    std::vector<std::filesystem::path> components;
    for (const auto& component : relative)
        components.push_back(component);
    if (components.empty() || components.size() > 2)
        return false;
    const std::string region = components[0].string();
    if (region != "USA" && region != "EUR" && region != "JAP")
        return false;
    return components.size() == 1 || components[1] == "Card A" || components[1] == "Card B";
}

bool hash_bytes(XXH3_state_t* state, const void* bytes, const std::size_t size) {
    return XXH3_128bits_update(state, bytes, size) != XXH_ERROR;
}

bool hash_u64(XXH3_state_t* state, const std::uint64_t value) {
    std::array<std::uint8_t, 8> bytes{};
    for (std::size_t index = 0; index < bytes.size(); ++index)
        bytes[index] = static_cast<std::uint8_t>(value >> (index * 8));
    return hash_bytes(state, bytes.data(), bytes.size());
}

std::string digest_hex(XXH3_state_t* state) {
    const XXH128_hash_t hash = XXH3_128bits_digest(state);
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(sizeof(canonical.digest) * 2);
    for (const std::uint8_t byte : canonical.digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0x0f]);
    }
    return output;
}

}  // namespace

bool materialize_automation_card_fixture(const std::filesystem::path& sourceRoot,
    const std::filesystem::path& destinationCardRoot, AutomationCardFixtureResult& result,
    std::string& error) {
    result = {};
    error.clear();
    std::error_code filesystemError;
    const auto sourceStatus = std::filesystem::symlink_status(sourceRoot, filesystemError);
    if (filesystemError || !std::filesystem::is_directory(sourceStatus) ||
        std::filesystem::is_symlink(sourceStatus))
    {
        error = "automation card fixture root is not a real directory";
        return false;
    }
    const auto destinationStatus =
        std::filesystem::symlink_status(destinationCardRoot, filesystemError);
    if (filesystemError || !std::filesystem::is_directory(destinationStatus) ||
        std::filesystem::is_symlink(destinationStatus))
    {
        error = "automation data root is not a real directory";
        return false;
    }

    const std::filesystem::path canonicalSource =
        std::filesystem::canonical(sourceRoot, filesystemError);
    if (filesystemError) {
        error = "cannot resolve automation card fixture root";
        return false;
    }
    const std::filesystem::path canonicalDestination =
        std::filesystem::canonical(destinationCardRoot, filesystemError);
    if (filesystemError || path_is_within(canonicalSource, canonicalDestination) ||
        path_is_within(canonicalDestination, canonicalSource))
    {
        error = "automation card fixture root overlaps its destination";
        return false;
    }
    if (!std::filesystem::is_empty(canonicalDestination, filesystemError) || filesystemError) {
        error = "automation card fixture destination is not empty";
        return false;
    }

    std::vector<FixtureFile> files;
    for (std::filesystem::recursive_directory_iterator
             iterator(canonicalSource, std::filesystem::directory_options::none, filesystemError),
        end;
        iterator != end; iterator.increment(filesystemError))
    {
        if (filesystemError) {
            error = "cannot enumerate automation card fixture";
            return false;
        }
        const auto status = iterator->symlink_status(filesystemError);
        if (filesystemError || std::filesystem::is_symlink(status) ||
            (!std::filesystem::is_directory(status) && !std::filesystem::is_regular_file(status)))
        {
            error = "automation card fixture contains an unsupported filesystem entry";
            return false;
        }
        const std::filesystem::path relative =
            std::filesystem::relative(iterator->path(), canonicalSource, filesystemError);
        if (filesystemError) {
            error = "cannot resolve an automation card fixture entry";
            return false;
        }
        if (std::filesystem::is_directory(status)) {
            if (!valid_fixture_directory(relative)) {
                error = "automation card fixture contains an invalid directory layout";
                return false;
            }
            continue;
        }
        const std::uint64_t size = iterator->file_size(filesystemError);
        if (filesystemError || !valid_fixture_path(relative) || size == 0 ||
            size > AutomationCardFixtureMaximumBytes ||
            files.size() == AutomationCardFixtureMaximumFiles ||
            result.byteCount > AutomationCardFixtureMaximumBytes - size)
        {
            error = "automation card fixture has an invalid path, size, or file count";
            return false;
        }
        const auto keyBytes = relative.generic_u8string();
        files.push_back({
            .source = iterator->path(),
            .relative = relative,
            .key = std::string(reinterpret_cast<const char*>(keyBytes.data()), keyBytes.size()),
            .size = size,
        });
        result.byteCount += size;
    }
    if (filesystemError || files.empty()) {
        error = "automation card fixture contains no GCI files";
        return false;
    }
    std::ranges::sort(files, {}, &FixtureFile::key);
    if (std::ranges::adjacent_find(files, [](const FixtureFile& left, const FixtureFile& right) {
            return left.key == right.key;
        }) != files.end())
    {
        error = "automation card fixture contains duplicate canonical paths";
        return false;
    }

    for (const FixtureFile& file : files) {
        const std::filesystem::path destination = canonicalDestination / file.relative;
        filesystemError.clear();
        if (!path_is_within(destination, canonicalDestination) ||
            std::filesystem::exists(destination, filesystemError) || filesystemError)
        {
            error = "automation card fixture destination is not fresh";
            return false;
        }
    }

    XXH3_state_t* state = XXH3_createState();
    if (state == nullptr || XXH3_128bits_reset(state) == XXH_ERROR) {
        if (state != nullptr)
            XXH3_freeState(state);
        error = "cannot initialize automation card fixture identity";
        return false;
    }
    if (!hash_bytes(state, IdentityDomain.data(), IdentityDomain.size())) {
        XXH3_freeState(state);
        error = "cannot initialize automation card fixture identity";
        return false;
    }
    std::vector<std::filesystem::path> temporaryFiles;
    bool success = true;
    std::array<char, 64 * 1024> buffer{};
    for (const FixtureFile& file : files) {
        if (!hash_u64(state, file.key.size()) ||
            !hash_bytes(state, file.key.data(), file.key.size()) || !hash_u64(state, file.size))
        {
            success = false;
            break;
        }
        const std::filesystem::path destination = canonicalDestination / file.relative;
        filesystemError.clear();
        std::filesystem::create_directories(destination.parent_path(), filesystemError);
        if (filesystemError) {
            success = false;
            break;
        }
        std::filesystem::path temporary = destination;
        temporary += ".dusk-fixture-tmp";
        filesystemError.clear();
        if (std::filesystem::exists(temporary, filesystemError) || filesystemError) {
            success = false;
            break;
        }
        temporaryFiles.push_back(temporary);
        std::ifstream input(file.source, std::ios::binary);
        std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
        if (!input || !output) {
            success = false;
            break;
        }
        std::uint64_t copied = 0;
        while (input && copied < file.size) {
            const std::streamsize requested = static_cast<std::streamsize>(
                std::min<std::uint64_t>(buffer.size(), file.size - copied));
            input.read(buffer.data(), requested);
            const std::streamsize count = input.gcount();
            if (count <= 0) {
                success = false;
                break;
            }
            output.write(buffer.data(), count);
            if (!hash_bytes(state, buffer.data(), static_cast<std::size_t>(count))) {
                success = false;
                break;
            }
            copied += static_cast<std::uint64_t>(count);
        }
        output.flush();
        if (!success || copied != file.size || input.peek() != std::char_traits<char>::eof() ||
            !output)
        {
            success = false;
            break;
        }
    }
    if (success) {
        for (std::size_t index = 0; index < files.size(); ++index) {
            filesystemError.clear();
            std::filesystem::rename(temporaryFiles[index],
                canonicalDestination / files[index].relative, filesystemError);
            if (filesystemError) {
                success = false;
                break;
            }
        }
    }
    if (!success) {
        for (const auto& temporary : temporaryFiles) {
            std::error_code ignored;
            std::filesystem::remove(temporary, ignored);
        }
        for (const FixtureFile& file : files) {
            std::error_code ignored;
            std::filesystem::remove(canonicalDestination / file.relative, ignored);
        }
        for (const std::string_view region : {"USA", "EUR", "JAP"}) {
            for (const std::string_view card : {"Card A", "Card B"}) {
                std::error_code ignored;
                std::filesystem::remove(canonicalDestination / region / card, ignored);
            }
            std::error_code ignored;
            std::filesystem::remove(canonicalDestination / region, ignored);
        }
        XXH3_freeState(state);
        error = "cannot materialize automation card fixture atomically";
        return false;
    }
    result.identity = "card-fixture:xxh3-128:" + digest_hex(state);
    result.fileCount = files.size();
    XXH3_freeState(state);
    return true;
}

void set_active_automation_card_fixture_identity(std::string identity) {
    sActiveIdentity = std::move(identity);
}

std::string_view active_automation_card_fixture_identity() {
    return sActiveIdentity;
}

}  // namespace dusk::automation
