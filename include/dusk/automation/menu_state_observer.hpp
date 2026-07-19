#pragma once

#include <cstdint>

namespace dusk::automation {

struct TitleMenuObservation {
    bool present = false;
    std::uint8_t procedure = 0xff;
    bool logoSkipReady = false;
    bool startReady = false;
};

struct NameSceneMenuObservation {
    bool present = false;
    std::uint8_t procedure = 0xff;
    bool fileSelectPresent = false;
    std::uint8_t fileSelectProcedure = 0xff;
    std::uint8_t cardCheckProcedure = 0xff;
};

// Observer-only access to menu state that is otherwise private to native game
// actors. This API copies state and never mutates game memory.
class MenuStateObserver {
public:
    [[nodiscard]] static TitleMenuObservation captureTitle();
    [[nodiscard]] static NameSceneMenuObservation captureNameScene();
};

}  // namespace dusk::automation
