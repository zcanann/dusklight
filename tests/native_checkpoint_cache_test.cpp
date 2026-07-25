#include "dusk/automation/native_checkpoint_cache.hpp"

#include <cstddef>
#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

using namespace dusk::automation;

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "native_checkpoint_cache_test.cpp:" << line
                  << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

StateCheckpointImage image(const std::string& identity, const std::size_t bytes) {
    return StateCheckpointImage{
        .entries = {{
            .name = "machine",
            .kind = StateCheckpointEntryKind::MemoryRegion,
            .bytes = std::vector<std::byte>(bytes),
        }},
        .digest = identity,
    };
}

void test_bounded_lru_and_accounting() {
    NativeCheckpointCache<std::string> cache(30, 2);
    REQUIRE(cache.insert("a", "sa", image("a", 8), "host-a", 2));
    REQUIRE(cache.insert("b", "sb", image("b", 8), "host-b", 2));
    REQUIRE(cache.find("a") != nullptr);

    std::vector<std::string> evicted;
    REQUIRE(cache.insert("c", "sc", image("c", 8), "host-c", 2, &evicted));
    REQUIRE(evicted == std::vector<std::string>{"b"});
    REQUIRE(cache.peek("a") != nullptr);
    REQUIRE(cache.peek("b") == nullptr);
    REQUIRE(cache.peek("c") != nullptr);

    const NativeCheckpointCacheStats stats = cache.stats();
    REQUIRE(stats.capacityBytes == 30);
    REQUIRE(stats.capacityEntries == 2);
    REQUIRE(stats.residentBytes == 20);
    REQUIRE(stats.residentCheckpointBytes == 16);
    REQUIRE(stats.residentMetadataBytes == 4);
    REQUIRE(stats.residentEntries == 2);
    REQUIRE(stats.insertions == 3);
    REQUIRE(stats.evictions == 1);
    REQUIRE(stats.hits == 1);
    REQUIRE(stats.misses == 0);
}

void test_replacement_and_oversize_fail_closed() {
    NativeCheckpointCache<std::string> cache(20, 2);
    REQUIRE(cache.insert("a", "sa", image("a", 8), "host-a", 2));
    REQUIRE(cache.insert("a", "sa2", image("a", 12), "host-a2", 3));
    REQUIRE(cache.stats().replacements == 1);
    REQUIRE(cache.stats().residentBytes == 15);
    REQUIRE(cache.peek("a")->semanticDigest == "sa2");

    REQUIRE(!cache.insert("huge", "sh", image("huge", 20), "host", 1));
    REQUIRE(!cache.insert("detached", "sd", image("different", 1), "host", 1));
    REQUIRE(cache.peek("a") != nullptr);
    REQUIRE(cache.stats().residentEntries == 1);
}

void test_misses_are_measured() {
    NativeCheckpointCache<std::string> cache(10, 1);
    REQUIRE(cache.find("absent") == nullptr);
    REQUIRE(cache.stats().misses == 1);
}

}  // namespace

int main() {
    test_bounded_lru_and_accounting();
    test_replacement_and_oversize_fail_closed();
    test_misses_are_measured();
    std::cout << "native checkpoint cache tests passed\n";
    return 0;
}
