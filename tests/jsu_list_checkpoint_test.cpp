#include "JSystem/JSupport/JSUList.h"

#include <cstdlib>
#include <iostream>

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "jsu_list_checkpoint_test.cpp:" << line << ": check failed: " << expression
                  << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

struct Item {
    explicit Item(const int value) : value(value), link(this) {}

    int value;
    JSULink<Item> link;
};

struct LinkImage {
    void* object;
    JSUPtrList* list;
    JSUPtrLink* previous;
    JSUPtrLink* next;
};

LinkImage capture_link(const JSUPtrLink& link) {
    return {link.mObject, link.mList, link.mPrev, link.mNext};
}

void restore_link(JSUPtrLink& link, const LinkImage& image) {
    link.mObject = image.object;
    link.mList = image.list;
    link.mPrev = image.previous;
    link.mNext = image.next;
}

void test_list_header_rewinds_after_backing_links() {
    JSUList<Item> list;
    Item first(1);
    Item second(2);
    Item later(3);
    REQUIRE(list.append(&first.link));
    REQUIRE(list.append(&second.link));

    const JSUPtrListCheckpointState source = list.captureCheckpointState();
    const LinkImage firstSource = capture_link(first.link);
    const LinkImage secondSource = capture_link(second.link);
    REQUIRE(list.append(&later.link));
    REQUIRE(list.getNumLinks() == 3);

    // MEM1 is restored before the native JSystem registry header.
    restore_link(first.link, firstSource);
    restore_link(second.link, secondSource);
    REQUIRE(list.restoreCheckpointState(source));
    REQUIRE(list.getNumLinks() == 2);
    REQUIRE(list.getFirst()->getObject() == &first);
    REQUIRE(list.getLast()->getObject() == &second);
    REQUIRE(first.link.getPrev() == nullptr);
    REQUIRE(first.link.getNext() == &second.link);
    REQUIRE(second.link.getPrev() == &first.link);
    REQUIRE(second.link.getNext() == nullptr);

    // The post-checkpoint object is no longer registered after its containing
    // emulated memory has been rewound.
    later.link.mList = nullptr;
    later.link.mPrev = nullptr;
    later.link.mNext = nullptr;
}

void test_invalid_headers_fail_closed() {
    JSUList<Item> list;
    Item item(1);
    REQUIRE(!list.restoreCheckpointState({&item.link, nullptr, 1}));
    REQUIRE(!list.restoreCheckpointState({nullptr, nullptr, 1}));
    REQUIRE(!list.restoreCheckpointState({&item.link, &item.link, 2}));
}

}  // namespace

int main() {
    test_list_header_rewinds_after_backing_links();
    test_invalid_headers_fail_closed();
    std::cout << "JSU list checkpoint tests passed\n";
    return 0;
}
