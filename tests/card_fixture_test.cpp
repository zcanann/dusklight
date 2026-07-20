#include "dusk/automation/card_fixture.hpp"

#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

namespace {

void require(const bool condition, const std::string_view message) {
    if (!condition) {
        std::cerr << "card fixture test failed: " << message << '\n';
        std::exit(1);
    }
}

void write_file(const std::filesystem::path& path, const std::string_view bytes) {
    std::filesystem::create_directories(path.parent_path());
    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    if (!stream || !stream.write(bytes.data(), static_cast<std::streamsize>(bytes.size())))
        throw std::runtime_error("could not create test fixture file");
}

std::string read_file(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

struct TemporaryDirectory {
    std::filesystem::path path;

    ~TemporaryDirectory() {
        std::error_code ignored;
        std::filesystem::remove_all(path, ignored);
    }
};

}  // namespace

int main() {
    const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
    TemporaryDirectory temporary{
        std::filesystem::temp_directory_path() /
            ("dusklight-card-fixture-test-" + std::to_string(nonce)),
    };
    std::filesystem::create_directories(temporary.path);

    const auto sourceA = temporary.path / "source-a";
    const auto sourceB = temporary.path / "source-b";
    const auto destinationA = temporary.path / "destination-a";
    const auto destinationB = temporary.path / "destination-b";
    std::filesystem::create_directories(destinationA);
    std::filesystem::create_directories(destinationB);

    // Creation order and filesystem metadata are deliberately different. Only
    // canonical relative paths and initial bytes participate in identity.
    write_file(sourceA / "USA" / "Card B" / "secondary.gci", "secondary-card-bytes");
    write_file(sourceA / "USA" / "Card A" / "zelda.gci", "primary-card-bytes");
    write_file(sourceB / "USA" / "Card A" / "zelda.gci", "primary-card-bytes");
    write_file(sourceB / "USA" / "Card B" / "secondary.gci", "secondary-card-bytes");

    dusk::automation::AutomationCardFixtureResult resultA;
    dusk::automation::AutomationCardFixtureResult resultB;
    std::string error;
    require(dusk::automation::materialize_automation_card_fixture(
                sourceA, destinationA, resultA, error),
        error);
    require(dusk::automation::materialize_automation_card_fixture(
                sourceB, destinationB, resultB, error),
        error);
    require(resultA.identity == resultB.identity,
        "equivalent fixture trees did not produce the same identity");
    require(resultA.identity.starts_with("card-fixture:xxh3-128:") &&
                resultA.identity.size() == std::string_view("card-fixture:xxh3-128:").size() + 32,
        "fixture identity is not canonical XXH3-128 text");
    require(
        resultA.fileCount == 2 && resultA.byteCount == 38, "fixture result counts are incorrect");
    require(read_file(destinationA / "USA" / "Card A" / "zelda.gci") == "primary-card-bytes",
        "primary fixture bytes changed during materialization");
    require(read_file(destinationA / "USA" / "Card B" / "secondary.gci") == "secondary-card-bytes",
        "secondary fixture bytes changed during materialization");

    dusk::automation::set_active_automation_card_fixture_identity(resultA.identity);
    require(dusk::automation::active_automation_card_fixture_identity() == resultA.identity,
        "active fixture identity was not retained");

    dusk::automation::AutomationCardFixtureResult rejected;
    require(!dusk::automation::materialize_automation_card_fixture(
                sourceA, destinationA, rejected, error) &&
                error == "automation card fixture destination is not empty",
        "a populated destination did not fail closed");

    const auto invalidSource = temporary.path / "invalid-source";
    const auto invalidDestination = temporary.path / "invalid-destination";
    std::filesystem::create_directories(invalidDestination);
    write_file(invalidSource / "USA" / "unexpected" / "zelda.gci", "bytes");
    require(!dusk::automation::materialize_automation_card_fixture(
                invalidSource, invalidDestination, rejected, error) &&
                error == "automation card fixture contains an invalid directory layout",
        "an unsupported card directory was accepted");

    const auto emptySource = temporary.path / "empty-source";
    const auto emptyDestination = temporary.path / "empty-destination";
    std::filesystem::create_directories(emptySource);
    std::filesystem::create_directories(emptyDestination);
    require(!dusk::automation::materialize_automation_card_fixture(
                emptySource, emptyDestination, rejected, error) &&
                error == "automation card fixture contains no GCI files",
        "an empty fixture was accepted as a populated fixture");

    const auto overlapSource = temporary.path / "overlap-source";
    const auto overlapDestination = overlapSource / "destination";
    write_file(overlapSource / "USA" / "Card A" / "zelda.gci", "bytes");
    std::filesystem::create_directories(overlapDestination);
    require(!dusk::automation::materialize_automation_card_fixture(
                overlapSource, overlapDestination, rejected, error) &&
                error == "automation card fixture root overlaps its destination",
        "a destination nested below its source was accepted");

    return 0;
}
