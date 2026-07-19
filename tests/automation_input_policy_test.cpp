#include "automation_input_policy.hpp"

#include <cstdlib>
#include <iostream>

namespace {

SDL_Event key_event(const SDL_EventType type, const SDL_Keycode key) {
    SDL_Event event{};
    event.type = type;
    event.key.key = key;
    return event;
}

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "automation_input_policy_test.cpp:" << line
                  << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

}  // namespace

int main() {
    using aurora::window::is_automation_debug_ui_key_event;

    REQUIRE(is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_DOWN, SDLK_F1)));
    REQUIRE(is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_UP, SDLK_F1)));
    REQUIRE(is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_DOWN, SDLK_LSHIFT)));
    REQUIRE(is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_UP, SDLK_RSHIFT)));

    REQUIRE(!is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_DOWN, SDLK_F2)));
    REQUIRE(!is_automation_debug_ui_key_event(key_event(SDL_EVENT_KEY_DOWN, SDLK_A)));

    SDL_Event mouse_event{};
    mouse_event.type = SDL_EVENT_MOUSE_BUTTON_DOWN;
    REQUIRE(!is_automation_debug_ui_key_event(mouse_event));

    SDL_Event gamepad_event{};
    gamepad_event.type = SDL_EVENT_GAMEPAD_BUTTON_DOWN;
    REQUIRE(!is_automation_debug_ui_key_event(gamepad_event));
}
