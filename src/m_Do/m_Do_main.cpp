/**
 * m_Do_main.cpp
 * Main Initialization
 * PC Port Version - based on Aurora integration from Vorversion
 */

#include "m_Do/m_Do_main.h"
#include <dolphin/vi.h>
#include <cstring>
#include "DynamicLink.h"
#include "JSystem/JAudio2/JASAudioThread.h"
#include "JSystem/JAudio2/JAUSectionHeap.h"
#include "JSystem/JAudio2/JAUSoundTable.h"
#include "JSystem/JFramework/JFWSystem.h"
#include "JSystem/JHostIO/JORServer.h"
#include "JSystem/JKernel/JKRAram.h"
#include "JSystem/JKernel/JKRSolidHeap.h"
#include "JSystem/JUtility/JUTConsole.h"
#include "JSystem/JUtility/JUTException.h"
#include "JSystem/JUtility/JUTGamePad.h"
#include "JSystem/JUtility/JUTProcBar.h"
#include "JSystem/JUtility/JUTReport.h"
#include "SSystem/SComponent/c_counter.h"
#include "SSystem/SComponent/c_API_graphic.h"
#include "Z2AudioLib/Z2WolfHowlMgr.h"
#include "c/c_dylink.h"
#include "d/d_com_inf_game.h"
#include "d/d_debug_pad.h"
#include "d/d_s_logo.h"
#include "d/d_s_menu.h"
#include "d/d_s_play.h"
#include "d/actor/d_a_alink.h"
#include "dusk/time.h"
#include "f_ap/f_ap_game.h"
#include "f_op/f_op_msg.h"
#include "f_op/f_op_actor_iter.h"
#include "m_Do/m_Do_MemCard.h"
#include "m_Do/m_Do_Reset.h"
#include "m_Do/m_Do_controller_pad.h"
#include "m_Do/m_Do_dvd_thread.h"
#include "m_Do/m_Do_ext2.h"
#include "m_Do/m_Do_graphic.h"
#include "m_Do/m_Do_machine.h"
#include "m_Do/m_Do_printf.h"
#include "m_Do/m_Do_ext2.h"
#include "SSystem/SComponent/c_counter.h"
#include <cstring>
#include <sstream>

#include <filesystem>
#include <cmath>
#include <limits>
#include <system_error>
#include <thread>
#include "SSystem/SComponent/c_API.h"
#include "dusk/android_frame_rate.hpp"
#include "dusk/app_info.hpp"
#include "dusk/automation/gameplay_trace.hpp"
#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/io_mode.hpp"
#include "dusk/automation/milestones.hpp"
#include "dusk/automation/rng.hpp"
#include "dusk/automation/eye_shredder_oracle.hpp"
#include "dusk/automation/name_entry_trace.hpp"
#include "dusk/automation/worker.hpp"
#include "dusk/crash_handler.h"
#include "dusk/crash_reporting.h"
#include "dusk/data.hpp"
#include "dusk/dusk.h"
#include "dusk/frame_interpolation.h"
#include "dusk/game_clock.h"
#include "dusk/gyro.h"
#include "dusk/mouse.h"
#include "dusk/imgui/ImGuiConsole.hpp"
#include "dusk/imgui/ImGuiEngine.hpp"
#include "dusk/iso_validate.hpp"
#include "dusk/mod_loader.hpp"
#include "dusk/logging.h"
#include "dusk/main.h"
#include "dusk/ui/menu_bar.hpp"
#include "dusk/ui/overlay.hpp"
#include "dusk/ui/prelaunch.hpp"
#include "dusk/ui/preset.hpp"
#include "dusk/ui/touch_controls.hpp"
#include "dusk/ui/ui.hpp"
#include "version.h"

#include <aurora/aurora.h>
#include <aurora/event.h>
#include <aurora/gfx.h>
#include <aurora/main.h>
#include <aurora/dvd.h>
#include <dolphin/card.h>
#include <dolphin/dvd.h>

#include "SDL3/SDL_init.h"
#include "SDL3/SDL_iostream.h"
#include "SDL3/SDL_misc.h"
#include "SDL3/SDL_video.h"
#include "cxxopts.hpp"
#include "d/actor/d_a_movie_player.h"
#include "dusk/audio/DuskAudioSystem.h"
#include "dusk/audio/DuskDsp.hpp"
#include "dusk/config.hpp"
#include "dusk/speedrun.h"
#include "dusk/settings.h"
#include "dusk/scope_guard.hpp"
#include "dusk/texture_replacements.hpp"
#include "dusk/io.hpp"
#include "dusk/version.hpp"
#include "dusk/discord_presence.hpp"
#include "tracy/Tracy.hpp"
#include "f_pc/f_pc_draw.h"
#include "tracy/Tracy.hpp"
#include <RmlUi/Core.h>
#ifdef __APPLE__
#include <TargetConditionals.h>
#endif

#if DUSK_ENABLE_SENTRY_NATIVE
#include "dusk/ui/reporting.hpp"
#endif

// --- GLOBALS ---
s8 mDoMain::developmentMode = -1;
OSTime mDoMain::sPowerOnTime;
OSTime mDoMain::sHungUpTime;
u32 mDoMain::memMargin = 0xFFFFFFFF;
char mDoMain::COPYDATE_STRING[18] = "??/??/?? ??:??:??";
#if TARGET_PC
const int audioHeapSize = 0x14D800 * 2;
#else
const int audioHeapSize = 0x14D800;
#endif

// =========================================================================
// LOAD_COPYDATE - PC Version
// =========================================================================
#define COPYDATE_PATH "/str/Final/Release/COPYDATE"

#if TARGET_PC
bool dusk::IsRunning = true;
bool dusk::IsShuttingDown = false;
bool dusk::IsGameLaunched = false;
bool dusk::RestartRequested = false;
uint8_t dusk::SaveRequested = 0;
dusk::StageRequest dusk::StageRequested = {"",false};
std::filesystem::path dusk::ConfigPath;
std::filesystem::path dusk::CachePath;
static bool automationInputQuarantine;
#endif

void dusk::RequestRestart() noexcept {
    RestartRequested = SupportsProcessRestart;
    IsRunning = false;
}

s32 LOAD_COPYDATE(void*) {
    char buffer[32];
    memset(buffer, 0, sizeof(buffer));

    DVDFileInfo fi;
    if (DVDOpen(COPYDATE_PATH, &fi)) {
        u32 readLen = (fi.length < sizeof(buffer) - 1) ? fi.length : sizeof(buffer) - 1;
        // DVDReadPrio requires 32-byte aligned buffer and length rounded up to 32
        u32 alignedLen = (readLen + 31) & ~31;
        alignas(32) char readBuf[64];
        DVDReadPrio(&fi, readBuf, alignedLen, 0, 2);
        DVDClose(&fi);

        memcpy(buffer, readBuf, readLen);
        buffer[readLen] = '\0';
    } else {
        SAFE_STRCPY(buffer, "PC PORT BUILD");
        DuskLog.warn("COPYDATE file not found at {}", COPYDATE_PATH);
    }

    memcpy(mDoMain::COPYDATE_STRING, buffer, sizeof(mDoMain::COPYDATE_STRING) - 1);
    mDoMain::COPYDATE_STRING[sizeof(mDoMain::COPYDATE_STRING) - 1] = '\0';

    DuskLog.info("COPYDATE=[{}]", mDoMain::COPYDATE_STRING);
    return 1;
}

AuroraInfo auroraInfo;
AuroraStats dusk::lastFrameAuroraStats;
float dusk::frameUsagePct = 0.0f;

static void finish_automation_renderer_frame();

bool launchUILoop() {
    while (dusk::IsRunning && !dusk::IsGameLaunched) {
        const AuroraEvent* event = aurora_update();
        while (event != nullptr && event->type != AURORA_NONE) {
            switch (event->type) {
            case AURORA_SDL_EVENT:
                dusk::mouse::handle_event(event->sdl);
                dusk::ui::handle_event(event->sdl);
                dusk::g_imguiConsole.HandleSDLEvent(event->sdl);
                break;
            case AURORA_DISPLAY_SCALE_CHANGED:
                dusk::ImGuiEngine_Initialize(event->windowSize.scale);
                break;
            case AURORA_EXIT:
                return false;
            }

            event++;
        }

        if (!aurora_begin_frame()) {
            DuskLog.debug("aurora_begin_frame returned false, skipping draw this frame");
            continue;
        }

        dusk::ui::update();

        dusk::g_imguiConsole.PreDraw();
        dusk::g_imguiConsole.PostDraw();

        aurora_end_frame();
        finish_automation_renderer_frame();
    }

    return dusk::IsRunning;
}

static bool finish_input_tape_tick();
static bool finish_simulation_tick();
static void begin_automation_simulation_tick();
static bool finish_automation_oracle_tick();
static bool automation_oracle_rejected_before_loop();
static bool unpacedMainLoop;
static bool fixedStepMainLoop;
static void write_automation_oracle_result_on_exit();
static void write_name_entry_trace_on_exit();
static void write_gameplay_trace_on_exit();
static void write_milestone_result_on_exit();

void main01(void) {
    OS_REPORT("\x1b[m");

    // 1. Setup
    mDoMch_Create();
    mDoGph_Create();
    mDoCPd_c::create();

    // Console Setup
    JUTConsole* console = JFWSystem::getSystemConsole();
    if (console) {
        console->setOutput(mDoMain::developmentMode ? JUTConsole::OUTPUT_OSR_AND_CONSOLE :
                                                      JUTConsole::OUTPUT_NONE);
        console->setPosition(32, 42);
    }

    // Loader Init
    mDoDvdThd_callback_c::create((mDoDvdThd_callback_func)LOAD_COPYDATE, NULL);

    OSReport("Calling fapGm_Create()...\n");
    fapGm_Create();

    OSReport("Calling fopAcM_initManager()...\n");
    fopAcM_initManager();

    OSReport("Calling cDyl_InitAsync()...\n");
    cDyl_InitAsync();

    g_mDoAud_audioHeap = JKRCreateSolidHeap(audioHeapSize, JKRGetCurrentHeap(), false);
    JKRHEAP_NAME(g_mDoAud_audioHeap, "g_mDoAud_audioHeap");

    if (DUSK_AUDIO_DISABLED) {
        // Pretend the audio engine initialized already. This is a lie, but needed to boot.
        mDoAud_zelAudio_c::onInitFlag();
    }

    OSReport("Entering Main Loop (main01)...\n");

    dusk::game_clock::ensure_initialized();

    if (automation_oracle_rejected_before_loop()) {
        goto exit;
    }

    do {
        // 1. Update Window Events
        const AuroraEvent* event = aurora_update();
        while (true) {
            switch (event->type) {
            case AURORA_NONE:
                goto eventsDone;
            case AURORA_PAUSED:
                dusk::audio::SetPaused(true);
                dusk::mouse::on_focus_lost();
                break;
            case AURORA_UNPAUSED:
                dusk::audio::SetPaused(false);
                dusk::game_clock::reset_frame_timer();
                dusk::mouse::on_focus_gained();
                break;
            case AURORA_SDL_EVENT:
                dusk::mouse::handle_event(event->sdl);
                dusk::ui::handle_event(event->sdl);
                dusk::g_imguiConsole.HandleSDLEvent(event->sdl);
                break;
            case AURORA_WINDOW_RESIZED:
                if (dusk::getSettings().video.rememberWindowSize && !dusk::getSettings().video.enableFullscreen) {
                    dusk::getSettings().video.lastWindowWidth.setValue(event->windowSize.width);
                    dusk::getSettings().video.lastWindowHeight.setValue(event->windowSize.height);
                    dusk::config::save();
                }
                break;
            case AURORA_DISPLAY_SCALE_CHANGED:
                dusk::ImGuiEngine_Initialize(event->windowSize.scale);
                break;
            case AURORA_EXIT:
                goto exit;
            }

            event++;
        }

        eventsDone:;

        if (!aurora_begin_frame()) {
            DuskLog.debug("aurora_begin_frame returned false, skipping draw this frame");
            continue;
        }

        VIWaitForRetrace();

        dusk::lastFrameAuroraStats = *aurora_get_stats();
        mDoGph_gInf_c::updateRenderSize();

        dusk::ui::update();

        const auto pacing = dusk::game_clock::advance_main_loop();
        if (pacing.is_interpolating) {
            if (pacing.sim_ticks_to_run > 0) {
                dusk::frame_interp::begin_frame(dusk::getSettings().game.enableFrameInterpolation, true, 0.0f);
                dusk::frame_interp::set_ui_tick_pending(true);

                for (int sim_tick = 0; sim_tick < pacing.sim_ticks_to_run; ++sim_tick) {
                    dusk::frame_interp::begin_sim_tick();
                    mDoCPd_c::read();
                    begin_automation_simulation_tick();
                    if (!automationInputQuarantine) {
                        dusk::mouse::read();
                        dusk::gyro::read(pacing.sim_pace);
                    }
                    fapGm_Execute();
                    mDoAud_Execute();
                    dusk::game_clock::commit_sim_tick();
                    if (!finish_simulation_tick()) {
                        break;
                    }
                    if (finish_automation_oracle_tick()) {
                        break;
                    }
                    if (finish_input_tape_tick()) {
                        break;
                    }
                }
            }

            dusk::frame_interp::begin_frame(dusk::getSettings().game.enableFrameInterpolation, false,
                                            dusk::game_clock::sample_interpolation_step());
            dusk::frame_interp::interpolate();
            dusk::frame_interp::begin_presentation_camera();
            // run draw functions for anything specially marked to handle interp
            fpcM_DrawIterater((fpcM_DrawIteraterFunc)fpcM_Draw);
            cAPIGph_Painter();
            dusk::frame_interp::end_presentation_camera();
            dusk::frame_interp::set_ui_tick_pending(false);
        } else {
            dusk::frame_interp::begin_frame(dusk::FrameInterpMode::Off, true, 0.0f);
            dusk::frame_interp::set_ui_tick_pending(true);

            // Game Inputs
            mDoCPd_c::read();
            begin_automation_simulation_tick();
            if (!automationInputQuarantine) {
                dusk::mouse::read();
                dusk::gyro::read(pacing.presentation_dt_seconds);
            }

            // EXECUTE GAME LOGIC & RENDER
            // This calls mDoGph_Painter -> JFWDisplay -> GX Functions
            fapGm_Execute();

            mDoAud_Execute();
            if (finish_simulation_tick()) {
                if (!finish_automation_oracle_tick()) {
                    finish_input_tape_tick();
                }
            }
        }

        aurora_end_frame();
        finish_automation_renderer_frame();

        FrameMark;

#ifdef DUSK_DISCORD
        dusk::discord::run_callbacks();
        dusk::discord::update_presence();
#endif

        static Limiter main_loop_limiter;
        static double last_fps_setting = 0.0;
        static Limiter::duration_t target_ns = 0;

        double current_fps = 0.0;
        if (fixedStepMainLoop && !unpacedMainLoop) {
            current_fps = 30.0;
        } else if (!unpacedMainLoop &&
                   dusk::getSettings().game.enableFrameInterpolation.getValue() ==
                       dusk::FrameInterpMode::Capped &&
                   !dusk::getTransientSettings().skipFrameRateLimit) {
            current_fps = dusk::getSettings().video.maxFrameRate.getValue();
        }

        if (current_fps > 0.0) {
            ZoneScopedN("Frame limiter");
            if (current_fps != last_fps_setting) {
                last_fps_setting = current_fps;
                target_ns = static_cast<Limiter::duration_t>(1'000'000'000.0 / current_fps);
            }

            Limiter::duration_t sleepTime = main_loop_limiter.Sleep(target_ns);
            dusk::frameUsagePct = 100.0f * (1.0f - static_cast<float>(sleepTime) / static_cast<float>(target_ns));
        } else {
            main_loop_limiter.Reset();
        }
    } while (dusk::IsRunning);

    exit:;
    write_automation_oracle_result_on_exit();
    write_name_entry_trace_on_exit();
    write_gameplay_trace_on_exit();
    write_milestone_result_on_exit();
    dusk::mods::ModLoader::instance().shutdown();
    dusk::ui::shutdown();
}

static bool IsBackendAvailable(AuroraBackend backend) {
    if (backend == BACKEND_AUTO) {
        return true;
    }

    size_t availableBackendCount = 0;
    const AuroraBackend* availableBackends = aurora_get_available_backends(&availableBackendCount);
    for (size_t i = 0; i < availableBackendCount; ++i) {
        if (availableBackends[i] == backend) {
            return true;
        }
    }

    return false;
}

static AuroraBackend ResolveDesiredBackend(const cxxopts::ParseResult& parsedArgOptions) {
    AuroraBackend desiredBackend = BACKEND_AUTO;

    if (parsedArgOptions.count("backend") != 0) {
        const std::string backendArg = parsedArgOptions["backend"].as<std::string>();
        if (!dusk::try_parse_backend(backendArg, desiredBackend)) {
            fmt::print(stderr, "Unknown backend: {}\n", backendArg);
            exit(1);
        }
    } else if (!dusk::try_parse_backend(
                   static_cast<const std::string&>(dusk::getSettings().backend.graphicsBackend),
                   desiredBackend))
    {
        DuskLog.warn("Unknown configured backend '{}', falling back to Auto",
                     static_cast<const std::string&>(dusk::getSettings().backend.graphicsBackend));
        desiredBackend = BACKEND_AUTO;
    }

    if (!IsBackendAvailable(desiredBackend)) {
        DuskLog.warn("Requested backend '{}' is unavailable, falling back to Auto",
                     dusk::backend_name(desiredBackend));
        desiredBackend = BACKEND_AUTO;
    }

    return desiredBackend;
}

static void aurora_imgui_init_callback(const AuroraWindowSize* size) {
    dusk::ImGuiEngine_Initialize(size->scale);
    dusk::ImGuiEngine_AddTextures();
}

static void ApplyCVarOverrides(const cxxopts::OptionValue& option) {
    if (option.count() == 0) {
        return;
    }

    const auto& cVars = option.as<std::vector<std::string>>();
    for (const auto& cvarArg : cVars) {
        const auto sep = cvarArg.find('=');
        if (sep == std::string::npos) {
            DuskLog.fatal("--cvar argument has no '=': '{}'", cvarArg);
            continue;
        }

        const auto name = std::string_view(cvarArg).substr(0, sep);
        const auto value = std::string_view(cvarArg).substr(sep + 1);

        dusk::config::load_arg_override(name, value);
    }
}

static constexpr PADDefaultMapping defaultPadMapping = {
    .buttons = {
        {SDL_GAMEPAD_BUTTON_SOUTH, PAD_BUTTON_A},
        {SDL_GAMEPAD_BUTTON_EAST, PAD_BUTTON_B},
        {SDL_GAMEPAD_BUTTON_WEST, PAD_BUTTON_X},
        {SDL_GAMEPAD_BUTTON_NORTH, PAD_BUTTON_Y},
        {SDL_GAMEPAD_BUTTON_START, PAD_BUTTON_START},
        {SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER, PAD_TRIGGER_Z},
        {PAD_NATIVE_BUTTON_INVALID, PAD_TRIGGER_L},
        {PAD_NATIVE_BUTTON_INVALID, PAD_TRIGGER_R},
        {SDL_GAMEPAD_BUTTON_DPAD_UP, PAD_BUTTON_UP},
        {SDL_GAMEPAD_BUTTON_DPAD_DOWN, PAD_BUTTON_DOWN},
        {SDL_GAMEPAD_BUTTON_DPAD_LEFT, PAD_BUTTON_LEFT},
        {SDL_GAMEPAD_BUTTON_DPAD_RIGHT, PAD_BUTTON_RIGHT},
    },
    .axes = {
        {{SDL_GAMEPAD_AXIS_LEFTX, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_LEFT_X_POS},
        {{SDL_GAMEPAD_AXIS_LEFTX, AXIS_SIGN_NEGATIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_LEFT_X_NEG},
        // SDL's gamepad y-axis is inverted from GC's
        {{SDL_GAMEPAD_AXIS_LEFTY, AXIS_SIGN_NEGATIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_LEFT_Y_POS},
        {{SDL_GAMEPAD_AXIS_LEFTY, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_LEFT_Y_NEG},
        {{SDL_GAMEPAD_AXIS_RIGHTX, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_RIGHT_X_POS},
        {{SDL_GAMEPAD_AXIS_RIGHTX, AXIS_SIGN_NEGATIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_RIGHT_X_NEG},
        // see above
        {{SDL_GAMEPAD_AXIS_RIGHTY, AXIS_SIGN_NEGATIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_RIGHT_Y_POS},
        {{SDL_GAMEPAD_AXIS_RIGHTY, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_RIGHT_Y_NEG},
        {{SDL_GAMEPAD_AXIS_LEFT_TRIGGER, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_TRIGGER_L},
        {{SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, AXIS_SIGN_POSITIVE}, SDL_GAMEPAD_BUTTON_INVALID, PAD_AXIS_TRIGGER_R},
    },
};

static bool mainCalled = false;

static bool exitAfterInputTape;
static bool inputTapePlaybackFailed;
static bool automationInputHandedOff;
static bool headlessMainLoop;
static bool deterministicTimeAdvanceFailed;
static std::filesystem::path nameEntryTracePath;
static bool nameEntryTraceWriteFailed;
static std::filesystem::path gameplayTracePath;
static bool gameplayTraceWriteFailed;
static std::filesystem::path milestoneResultPath;
static bool milestoneResultWriteFailed;
static bool eyeShredderOracleEnabled;
static bool automationOracleContinueOnPass;
static dusk::automation::EyeShredderOracle eyeShredderOracle;
static std::filesystem::path eyeShredderOracleResultPath;
static bool eyeShredderOracleResultWriteFailed;
static std::uint64_t automationSimulationTick;
static std::uint64_t automationTapeFrame = dusk::automation::NameEntryNoTick;

static bool automation_oracle_rejected_before_loop() {
    return eyeShredderOracleEnabled && eyeShredderOracle.isTerminal() &&
           !(automationOracleContinueOnPass &&
             eyeShredderOracle.result().status ==
                 dusk::automation::EyeShredderOracleStatus::Passed);
}

static void begin_automation_simulation_tick() {
    const std::size_t nextFrame = dusk::automation::input_tape_player().nextFrameIndex();
    automationTapeFrame = nextFrame == 0
                              ? dusk::automation::NameEntryNoTick
                              : static_cast<std::uint64_t>(nextFrame - 1);
    dusk::automation::name_entry_observer().setTickContext(automationSimulationTick,
                                                            automationTapeFrame);
}

static void record_gameplay_trace_tick() {
    auto& recorder = dusk::automation::gameplay_trace_recorder();
    if (!recorder.active()) {
        return;
    }

    dusk::automation::GameplayTraceSample sample{
        .simulationTick = automationSimulationTick,
        .tapeFrame = automationTapeFrame,
        .room = static_cast<std::int8_t>(dComIfGp_roomControl_getStayNo()),
        .layer = static_cast<std::int8_t>(dComIfG_play_c::getLayerNo(0)),
        .point = dComIfGp_getStartStagePoint(),
    };
    if (const char* stageName = dComIfGp_getStartStageName(); stageName != nullptr) {
        std::strncpy(sample.stageName, stageName, sizeof(sample.stageName));
    }

    const auto& tapePlayer = dusk::automation::input_tape_player();
    if (tapePlayer.isPlaying()) {
        sample.flags |= dusk::automation::GameplayTraceTapePlaying;
    }
    const PADStatus& pad = JUTGamePad::mPadStatus[0];
    sample.buttons = pad.button;
    sample.stickX = pad.stickX;
    sample.stickY = pad.stickY;
    sample.padError = pad.err;
    if (dComIfGp_event_runCheck() != 0) {
        sample.flags |= dusk::automation::GameplayTraceEventRunning;
    }
    dEvt_control_c* event = dComIfGp_getEvent();
    sample.eventId = event->mEventId;
    sample.eventMode = event->getMode();
    sample.eventStatus = event->mEventStatus;
    sample.eventMapToolId = event->getMapToolId();
    const char* eventName = dComIfGp_getEventManager().getRunEventName();
    std::uint32_t eventNameHash = 2166136261u;
    for (const unsigned char* cursor = reinterpret_cast<const unsigned char*>(eventName);
         cursor != nullptr && *cursor != 0; ++cursor) {
        eventNameHash = (eventNameHash ^ *cursor) * 16777619u;
    }
    sample.eventNameHash = eventNameHash;

    if (fopAc_ac_c* player = dComIfGp_getPlayer(0); player != nullptr) {
        sample.flags |= dusk::automation::GameplayTracePlayerPresent;
        sample.playerActorName = fopAcM_GetName(player);
        if (sample.playerActorName == fpcNm_ALINK_e) {
            sample.flags |= dusk::automation::GameplayTracePlayerIsLink;
            sample.playerProcId = static_cast<daAlink_c*>(player)->mProcID;
        }
        sample.currentAngleY = player->current.angle.y;
        sample.shapeAngleY = player->shape_angle.y;
        sample.positionX = player->current.pos.x;
        sample.positionY = player->current.pos.y;
        sample.positionZ = player->current.pos.z;
        sample.velocityX = player->speed.x;
        sample.velocityY = player->speed.y;
        sample.velocityZ = player->speed.z;
        sample.forwardSpeed = player->speedF;

        struct NearestSceneExit {
            const cXyz* playerPosition;
            fopAc_ac_c* actor;
            float distanceSquared;
        } nearest{&player->current.pos, nullptr, std::numeric_limits<float>::max()};
        fopAcIt_Executor(
            [](void* candidate, void* context) -> int {
                auto* actor = static_cast<fopAc_ac_c*>(candidate);
                auto* nearest = static_cast<NearestSceneExit*>(context);
                const s16 actorName = fopAcM_GetName(actor);
                if (actorName != fpcNm_SCENE_EXIT_e && actorName != fpcNm_SCENE_EXIT2_e) {
                    return 1;
                }
                const float distance = actor->current.pos.abs2(*nearest->playerPosition);
                if (distance < nearest->distanceSquared) {
                    nearest->actor = actor;
                    nearest->distanceSquared = distance;
                }
                return 1;
            },
            &nearest);
        if (nearest.actor != nullptr) {
            sample.nearestSceneExitActorName = fopAcM_GetName(nearest.actor);
            sample.nearestSceneExitX = nearest.actor->current.pos.x;
            sample.nearestSceneExitY = nearest.actor->current.pos.y;
            sample.nearestSceneExitZ = nearest.actor->current.pos.z;
            sample.nearestSceneExitDistance = std::sqrt(nearest.distanceSquared);
        }
    }
    recorder.record(sample);
}

static bool record_milestone_tick() {
    auto& tracker = dusk::automation::milestone_tracker();
    if (!tracker.active()) {
        return false;
    }

    fopAc_ac_c* player = dComIfGp_getPlayer(0);
    const bool playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e;
    const auto* link = playerIsLink ? static_cast<daAlink_c*>(player) : nullptr;
    dEvt_control_c* event = dComIfGp_getEvent();
    const char* eventName = dComIfGp_getEventManager().getRunEventName();
    std::uint32_t eventNameHash = 0;
    if (eventName != nullptr) {
        eventNameHash = 2166136261u;
        for (const unsigned char* cursor = reinterpret_cast<const unsigned char*>(eventName);
             *cursor != 0; ++cursor) {
            eventNameHash = (eventNameHash ^ *cursor) * 16777619u;
        }
    }
    const bool goalReachedBefore = tracker.goalReached();
    tracker.observe(
        {
            .stageName = dComIfGp_getStartStageName(),
            .room = static_cast<std::int8_t>(dComIfGp_roomControl_getStayNo()),
            .layer = static_cast<std::int8_t>(dComIfG_play_c::getLayerNo(0)),
            .point = dComIfGp_getStartStagePoint(),
            .playerPresent = player != nullptr,
            .playerIsLink = playerIsLink,
            .playerProcessId = player == nullptr ? fpcM_ERROR_PROCESS_ID_e : fopAcM_GetID(player),
            .playerActorName = static_cast<std::int16_t>(
                player == nullptr ? -1 : fopAcM_GetName(player)),
            .playerProcId =
                static_cast<std::uint16_t>(link == nullptr ? 0xffff : link->mProcID),
            .playerPositionX = player == nullptr ? 0.0f : player->current.pos.x,
            .playerPositionY = player == nullptr ? 0.0f : player->current.pos.y,
            .playerPositionZ = player == nullptr ? 0.0f : player->current.pos.z,
            .playerVelocityX = player == nullptr ? 0.0f : player->speed.x,
            .playerVelocityY = player == nullptr ? 0.0f : player->speed.y,
            .playerVelocityZ = player == nullptr ? 0.0f : player->speed.z,
            .playerForwardSpeed = player == nullptr ? 0.0f : player->speedF,
            .playerCurrentAngleX =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.x),
            .playerCurrentAngleY =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.y),
            .playerCurrentAngleZ =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.z),
            .playerShapeAngleX =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.x),
            .playerShapeAngleY =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.y),
            .playerShapeAngleZ =
                static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.z),
            .eventRunning = dComIfGp_event_runCheck() != 0,
            .eventId = event->mEventId,
            .eventMode = event->getMode(),
            .eventStatus = event->mEventStatus,
            .eventMapToolId = event->getMapToolId(),
            .eventNameHash = eventNameHash,
            .nextStageEnabled = dComIfGp_isEnableNextStage() != 0,
            .nextStageName = dComIfGp_getNextStageName(),
            .nextRoom = static_cast<std::int8_t>(dComIfGp_getNextStageRoomNo()),
            .nextLayer = static_cast<std::int8_t>(dComIfGp_getNextStageLayer()),
            .nextPoint = dComIfGp_getNextStagePoint(),
            .rng = dusk::automation::capture_game_rng_snapshot(),
        },
        automationSimulationTick, automationTapeFrame);
    if (!goalReachedBefore && tracker.goalReached()) {
        DuskLog.info("Automation milestone goal '{}' reached at simulation tick {}, tape frame {}",
                     dusk::automation::milestone_name(*tracker.goal()),
                     automationSimulationTick, automationTapeFrame);
        return true;
    }
    return false;
}

static bool finish_automation_oracle_tick() {
    record_gameplay_trace_tick();
    const bool milestoneGoalReached = record_milestone_tick();
    if (!eyeShredderOracleEnabled) {
        ++automationSimulationTick;
        if (milestoneGoalReached) {
            dusk::IsRunning = false;
        }
        return milestoneGoalReached;
    }

    eyeShredderOracle.evaluate(dusk::automation::name_entry_observer().latest(),
                               automationSimulationTick, automationTapeFrame);
    fopAc_ac_c* player = dComIfGp_getPlayer(0);
    eyeShredderOracle.observeGameplayTelemetry(
        {
            .stageName = dComIfGp_getStartStageName(),
            .room = dComIfGp_getStartStageRoomNo(),
            .point = dComIfGp_getStartStagePoint(),
            .layer = dComIfGp_getStartStageLayer(),
            .playerActorName = player == nullptr ? -1 : fopAcM_GetName(player),
            .playerActorPresent = player != nullptr,
            .playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e,
            .eventRunning = dComIfGp_event_runCheck() != 0,
        },
        automationSimulationTick, automationTapeFrame);
    ++automationSimulationTick;
    if (milestoneGoalReached) {
        dusk::IsRunning = false;
        return true;
    }
    if (!eyeShredderOracle.isTerminal()) {
        return false;
    }

    if (automationOracleContinueOnPass &&
        eyeShredderOracle.result().status ==
            dusk::automation::EyeShredderOracleStatus::Passed)
    {
        return false;
    }

    dusk::IsRunning = false;
    return true;
}

static void finish_automation_renderer_frame() {
    if (!eyeShredderOracleEnabled) {
        return;
    }

    AuroraGXChannelCountTelemetry source{};
    aurora_get_gx_channel_count_telemetry(&source);
    const dusk::automation::EyeShredderRendererTelemetry telemetry{
        .xfNumChansRaw = source.lastMismatchXfNumChansRaw,
        .bpNumChansRaw = source.lastMismatchBpNumChansRaw,
        .mismatchLatched = source.mismatchLatched != 0,
        .eyeShredderMismatchLatched = source.eyeShredderMismatchLatched != 0,
        .mismatchDrawCount = source.mismatchDrawCount,
    };
    eyeShredderOracle.observeRendererTelemetry(
        telemetry, automationSimulationTick, automationTapeFrame);

    if (!automationOracleContinueOnPass &&
        eyeShredderOracle.result().status ==
            dusk::automation::EyeShredderOracleStatus::Passed)
    {
        dusk::IsRunning = false;
    }
}

static bool finish_input_tape_tick() {
    auto& player = dusk::automation::input_tape_player();
    if (player.isPlaying()) {
        return false;
    }

    if (player.hasFailed()) {
        const std::string reason = fmt::format(
            "{} at frame {} waiting for {}",
            dusk::automation::input_tape_playback_error_message(player.playbackError()),
            player.failedFrameIndex(),
            dusk::automation::input_frame_condition_name(player.failedCondition()));
        DuskLog.error("Input tape playback failed: {}", reason);
        inputTapePlaybackFailed = true;
        if (eyeShredderOracleEnabled && !eyeShredderOracle.isTerminal()) {
            eyeShredderOracle.reject(reason);
        }
        dusk::IsRunning = false;
        return true;
    }

    if (eyeShredderOracleEnabled) {
        eyeShredderOracle.observeTapeCompletion(automationSimulationTick, automationTapeFrame);
    }

    if (eyeShredderOracleEnabled && !eyeShredderOracle.isTerminal()) {
        eyeShredderOracle.finish(automationSimulationTick, automationTapeFrame);
        dusk::IsRunning = false;
        return true;
    }
    if (exitAfterInputTape) {
        dusk::IsRunning = false;
        return true;
    }
    if (!headlessMainLoop && !automationInputHandedOff) {
        player.handoffToLiveInput();
        PADPrepareAutomationHandoff();
        automationInputQuarantine = false;
        aurora_set_automation_input_quarantine(false);
        automationInputHandedOff = true;
        DuskLog.info("Input tape complete; live controller input resumed");
    }
    return false;
}

static void write_automation_oracle_result_on_exit() {
    if (!eyeShredderOracleEnabled) {
        return;
    }
    if (!eyeShredderOracle.isTerminal()) {
        eyeShredderOracle.finish(automationSimulationTick, automationTapeFrame);
    }

    std::string error;
    if (!dusk::automation::write_eye_shredder_oracle_result(
            eyeShredderOracleResultPath, eyeShredderOracle.result(), error)) {
        DuskLog.error("Failed to write Eye Shredder oracle result '{}': {}",
                      dusk::io::fs_path_to_string(eyeShredderOracleResultPath), error);
        eyeShredderOracleResultWriteFailed = true;
        return;
    }

    DuskLog.info("Eye Shredder oracle result: {} ({})",
                 dusk::automation::serialize_eye_shredder_oracle_result(
                     eyeShredderOracle.result()),
                 dusk::io::fs_path_to_string(eyeShredderOracleResultPath));
}

static bool finish_simulation_tick() {
    if (dusk::game_clock::complete_sim_tick()) {
        return true;
    }

    DuskLog.error("Deterministic OS time failed to advance after a completed simulation tick "
                  "(clock disabled or OSTime overflow)");
    deterministicTimeAdvanceFailed = true;
    dusk::IsRunning = false;
    return false;
}

static void write_name_entry_trace_on_exit() {
    if (nameEntryTracePath.empty()) {
        return;
    }

    auto artifact = dusk::automation::drain_name_entry_trace(
        dusk::automation::name_entry_observer());
    std::string error;
    if (!dusk::automation::write_name_entry_trace(nameEntryTracePath, artifact, error)) {
        DuskLog.error("Failed to write name-entry trace '{}': {}",
                      dusk::io::fs_path_to_string(nameEntryTracePath), error);
        nameEntryTraceWriteFailed = true;
        return;
    }

    DuskLog.info("Wrote name-entry trace '{}' ({} events, {} dropped)",
                 dusk::io::fs_path_to_string(nameEntryTracePath), artifact.events.size(),
                 artifact.droppedEventCount);
}

static void write_gameplay_trace_on_exit() {
    if (gameplayTracePath.empty()) {
        return;
    }

    auto& recorder = dusk::automation::gameplay_trace_recorder();
    recorder.stop();
    std::string error;
    if (!dusk::automation::write_gameplay_trace(gameplayTracePath, recorder, error)) {
        DuskLog.error("Failed to write gameplay trace '{}': {}",
                      dusk::io::fs_path_to_string(gameplayTracePath), error);
        gameplayTraceWriteFailed = true;
        return;
    }
    DuskLog.info("Wrote gameplay trace '{}' ({} samples{})",
                 dusk::io::fs_path_to_string(gameplayTracePath), recorder.samples().size(),
                 recorder.capacityExhausted() ? ", capacity exhausted" : "");
}

static void write_milestone_result_on_exit() {
    if (milestoneResultPath.empty()) {
        return;
    }

    auto& tracker = dusk::automation::milestone_tracker();
    std::string error;
    if (!dusk::automation::write_milestone_result(milestoneResultPath, tracker, error)) {
        DuskLog.error("Failed to write milestone result '{}': {}",
                      dusk::io::fs_path_to_string(milestoneResultPath), error);
        milestoneResultWriteFailed = true;
        return;
    }
    DuskLog.info("Wrote milestone result '{}' (goal reached: {})",
                 dusk::io::fs_path_to_string(milestoneResultPath), tracker.goalReached());
}

static u8 selectedLanguage;

u8 OSGetLanguage() {
    return selectedLanguage;
}

static void LanguageInit() {
    // Keep language at 0 (English) if not on a PAL disc.
    // Doubt this matters, but avoid funky shit.
    if (!dusk::version::isRegionPal()) {
        return;
    }

    // Cache this to avoid funky shenanigans.
    selectedLanguage = static_cast<u8>(dusk::getSettings().game.language.getValue());
}

static void log_build_info() {
    DuskLog.info("Build: {} (rev {}, built {}, type {})", DUSK_WC_DESCRIBE, DUSK_WC_REVISION, DUSK_WC_DATE, DUSK_BUILD_TYPE);
    DuskLog.info("Platform: {}", DUSK_PLATFORM_NAME);
}

// =========================================================================
// PC ENTRY POINT
// =========================================================================
int game_main(int argc, char* argv[]) {
    // On iOS, when connected to an external monitor, SDLUIKitSceneDelegate scene:willConnectToSession:
    // can call our main function again. Explicitly guard against this reinitialization.
    if (mainCalled) {
        return 0;
    }
    mainCalled = true;

    // Automation control modes intentionally run before settings, SDL, Aurora,
    // logging, or the game are initialized. This keeps the protocol usable by
    // process supervisors even on machines without a display or game image.
    if (const auto automationResult = dusk::automation::run_from_command_line(argc, argv)) {
        return *automationResult;
    }

    bool deterministicTimeEnabled = false;
    SimpleScopeGuard deterministicTimeGuard([&deterministicTimeEnabled] {
        if (deterministicTimeEnabled) {
            AuroraDisableDeterministicTime();
        }
    });

    cxxopts::ParseResult parsed_arg_options;
    std::string inputTapePath;
    std::size_t inputTapeFrameCount = 0;
    dusk::automation::TapeEndBehavior inputTapeEndBehavior =
        dusk::automation::TapeEndBehavior::Release;

    try {
        cxxopts::Options arg_options("Dusklight", "PC Port of a classic adventure game");

        arg_options.add_options()
            ("l,log-level", "Log level from " + std::to_string(AuroraLogLevel::LOG_DEBUG) + " to " + std::to_string(AuroraLogLevel::LOG_FATAL), cxxopts::value<uint8_t>()->default_value("0"))
            ("h,help", "Print usage")
            ("console", "Show the Windows console window for logs", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("dvd", "Path to DVD image file", cxxopts::value<std::string>())
            ("configured-dvd", "Open the last configured DVD image directly without showing prelaunch", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("mods", "Path to mods directory", cxxopts::value<std::string>())
            ("backend", "Graphics API backend to use (auto, d3d12, d3d11, metal, vulkan, null)", cxxopts::value<std::string>())
            ("cvar", "Override configuration variables without modifying config", cxxopts::value<std::vector<std::string>>())
            ("develop", "Enable the game's developer mode and OSReport for debugging", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("automation-hello", "Print the automation worker identity and capabilities as JSON, then exit")
            ("automation-worker", "Run the persistent automation control protocol over stdin/stdout")
            ("fixed-step", "Run exactly one deterministic 30 Hz logical tick per presented frame", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("unpaced", "Run exactly one 30 Hz logical tick per outer loop without frame pacing", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("headless", "Use the null render backend with an invisible window; implies --unpaced and requires --dvd", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("deterministic-time-start", "Initial signed OS timer tick for fixed-step modes (default 0)", cxxopts::value<std::int64_t>())
            ("input-tape", "Play a DUSKTAPE input file from the first game tick", cxxopts::value<std::string>())
            ("input-tape-end", "Input state after the tape ends (release, hold, loop)", cxxopts::value<std::string>()->default_value("release"))
            ("exit-after-tape", "Exit after the final tape frame executes", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("automation-data-root", "Isolate all writable Dusklight state for this tape run", cxxopts::value<std::string>())
            ("automation-card-root", "Use an explicit memory-card root for this tape run", cxxopts::value<std::string>())
            ("name-entry-trace", "Write a versioned name-entry observer trace when the game loop exits", cxxopts::value<std::string>())
            ("gameplay-trace", "Write compact per-tick stage, player motion, and input telemetry", cxxopts::value<std::string>())
            ("milestones", "Evaluate comma-separated memory-backed milestone IDs", cxxopts::value<std::string>())
            ("milestone-goal", "Stop on first hit of this requested milestone", cxxopts::value<std::string>())
            ("milestone-result", "Write versioned memory-backed milestone results as JSON", cxxopts::value<std::string>())
            ("cursor-breakout-shadow", "Model Cursor Breakout writes in bounded shadow memory (requires --name-entry-trace)", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("automation-oracle", "Run a semantic automation oracle (supported: eye-shredder)", cxxopts::value<std::string>())
            ("automation-oracle-result", "Write the semantic automation oracle result as versioned JSON", cxxopts::value<std::string>())
            ("automation-oracle-continue-on-pass", "Keep playing after an automation oracle passes; failures still stop immediately", cxxopts::value<bool>()->default_value("false")->implicit_value("true"))
            ("load-save", "Skip the opening and load a save from slot 1-3", cxxopts::value<uint8_t>()->default_value("0"))
            ("stage", "Upon launching, load a stage, room, spawn point, and layer. When using --load-save, it uses the specified save on the loaded stage. Format (STAGE,ROOM,POINT,LAYER). Example: (STAGE) or (STAGE,0,0,-1)", cxxopts::value<std::string>());

        arg_options.parse_positional({"dvd"});
        arg_options.positional_help("<dvd-image>");
        arg_options.allow_unrecognised_options();

        parsed_arg_options = arg_options.parse(argc, argv);

        if (parsed_arg_options.count("help"))
        {
            printf("%s", (arg_options.help() + "\n").c_str());
            exit(0);
        }

        if (parsed_arg_options.count("stage")) {
            std::stringstream ss(parsed_arg_options["stage"].as<std::string>());
            std::string token;

            std::getline(ss,token,',');
            std::string stageName = token;
            s8 room = 0;
            s16 point = 0;
            s8 layer = -1;
            if (std::getline(ss,token,',')) {
                room = std::stoi(token);
                if (std::getline(ss,token,',')) {
                    point = std::stoi(token);
                    if (std::getline(ss,token,',')) {
                        layer = std::stoi(token);
                    }
                }
            }

            dusk::StageRequested = {stageName,true, room,point,layer};
        }
    }
    catch (const cxxopts::exceptions::exception& e) {
        fprintf(stderr, "Argument Error: %s\n", e.what());
        exit(1);
    }
    catch (const std::invalid_argument& e) {
        // Handle parsing std::stoi when loading a stage
        fprintf(stderr, "Fatal: Invalid Argument When Parsing Stage\n");
        exit(1);
    }
    catch (const std::out_of_range& e) {
        // Handle parsing std::stoi when loading a stage
        fprintf(stderr, "Fatal: Argument Out of Range In Parsing Stage\n");
        exit(1);
    }

    headlessMainLoop = parsed_arg_options["headless"].as<bool>();
    unpacedMainLoop = headlessMainLoop || parsed_arg_options["unpaced"].as<bool>();
    fixedStepMainLoop = unpacedMainLoop || parsed_arg_options["fixed-step"].as<bool>();
    const bool useConfiguredDvd = parsed_arg_options["configured-dvd"].as<bool>();
    if (useConfiguredDvd && parsed_arg_options.count("dvd")) {
        fprintf(stderr, "DVD Error: --configured-dvd cannot be combined with --dvd PATH\n");
        return 1;
    }
    const bool hasDeterministicTimeStart = parsed_arg_options.count("deterministic-time-start") != 0;
    if (hasDeterministicTimeStart && !fixedStepMainLoop) {
        fprintf(stderr,
                "Time Error: --deterministic-time-start requires --fixed-step, --unpaced, or --headless\n");
        return 1;
    }
    if (headlessMainLoop && !parsed_arg_options.count("dvd")) {
        fprintf(stderr, "Headless Error: --headless requires an explicit --dvd PATH\n");
        return 1;
    }
    if (headlessMainLoop && parsed_arg_options.count("backend") &&
        parsed_arg_options["backend"].as<std::string>() != "null") {
        fprintf(stderr, "Headless Error: --headless only supports --backend null\n");
        return 1;
    }
    const OSTime deterministicInitialTicks = hasDeterministicTimeStart
                                                 ? parsed_arg_options["deterministic-time-start"].as<std::int64_t>()
                                                 : 0;
    if (fixedStepMainLoop) {
        if (!AuroraEnableDeterministicTime(deterministicInitialTicks, 30, 1)) {
            fprintf(stderr,
                    "Time Error: failed to enable deterministic OS time at tick %lld (30/1 Hz)\n",
                    static_cast<long long>(deterministicInitialTicks));
            return 1;
        }
        deterministicTimeEnabled = true;
    }
    dusk::game_clock::set_main_loop_mode(
        fixedStepMainLoop ? dusk::game_clock::MainLoopMode::FixedStep
                          : dusk::game_clock::MainLoopMode::Realtime);

    const bool cursorBreakoutShadow = parsed_arg_options["cursor-breakout-shadow"].as<bool>();
    const bool hasNameEntryTrace = parsed_arg_options.count("name-entry-trace") != 0;
    if (cursorBreakoutShadow && !hasNameEntryTrace) {
        fprintf(stderr,
                "Name Entry Error: --cursor-breakout-shadow requires --name-entry-trace PATH\n");
        return 1;
    }
    if (hasNameEntryTrace) {
        const std::string tracePath = parsed_arg_options["name-entry-trace"].as<std::string>();
        if (tracePath.empty()) {
            fprintf(stderr, "Name Entry Error: --name-entry-trace PATH cannot be empty\n");
            return 1;
        }
        nameEntryTracePath = std::filesystem::u8path(tracePath);
    }

    auto& nameEntryObserver = dusk::automation::name_entry_observer();
    nameEntryObserver.clearEvents();
    nameEntryObserver.setFidelityProfile(
        cursorBreakoutShadow
            ? dusk::automation::NameEntryFidelityProfile::CursorBreakoutShadow
            : dusk::automation::NameEntryFidelityProfile::ObserveOnly);

    const bool hasInputTape = parsed_arg_options.count("input-tape") != 0;
    exitAfterInputTape = parsed_arg_options["exit-after-tape"].as<bool>();

    if (parsed_arg_options.count("gameplay-trace")) {
        if (!hasInputTape) {
            fprintf(stderr, "Gameplay Trace Error: --gameplay-trace requires --input-tape PATH\n");
            return 1;
        }
        const std::string tracePath = parsed_arg_options["gameplay-trace"].as<std::string>();
        if (tracePath.empty()) {
            fprintf(stderr, "Gameplay Trace Error: --gameplay-trace PATH cannot be empty\n");
            return 1;
        }
        gameplayTracePath = std::filesystem::u8path(tracePath);
    }

    const bool hasMilestones = parsed_arg_options.count("milestones") != 0;
    const bool hasMilestoneResult = parsed_arg_options.count("milestone-result") != 0;
    const bool hasMilestoneGoal = parsed_arg_options.count("milestone-goal") != 0;
    if (hasMilestones != hasMilestoneResult) {
        fprintf(stderr,
                "Milestone Error: --milestones LIST and --milestone-result PATH must be used together\n");
        return 1;
    }
    if (hasMilestoneGoal && !hasMilestones) {
        fprintf(stderr, "Milestone Error: --milestone-goal ID requires --milestones LIST\n");
        return 1;
    }
    if (hasMilestones) {
        if (!hasInputTape) {
            fprintf(stderr, "Milestone Error: --milestones LIST requires --input-tape PATH\n");
            return 1;
        }

        std::vector<dusk::automation::MilestoneId> requestedMilestones;
        std::string milestoneError;
        if (!dusk::automation::parse_milestone_list(
                parsed_arg_options["milestones"].as<std::string>(), requestedMilestones,
                milestoneError)) {
            fprintf(stderr, "Milestone Error: %s\n", milestoneError.c_str());
            return 1;
        }

        std::optional<dusk::automation::MilestoneId> milestoneGoal;
        if (hasMilestoneGoal) {
            const std::string goalName =
                parsed_arg_options["milestone-goal"].as<std::string>();
            const auto* definition = dusk::automation::find_milestone(goalName);
            if (definition == nullptr) {
                fprintf(stderr, "Milestone Error: unknown goal '%s'\n", goalName.c_str());
                return 1;
            }
            milestoneGoal = definition->id;
        }

        const std::string resultPath =
            parsed_arg_options["milestone-result"].as<std::string>();
        if (resultPath.empty()) {
            fprintf(stderr, "Milestone Error: --milestone-result PATH cannot be empty\n");
            return 1;
        }
        if (!dusk::automation::milestone_tracker().configure(
                requestedMilestones, milestoneGoal, milestoneError)) {
            fprintf(stderr, "Milestone Error: %s\n", milestoneError.c_str());
            return 1;
        }
        milestoneResultPath = std::filesystem::u8path(resultPath);
    }

    std::filesystem::path automationCardRoot;
    std::filesystem::path automationDataRoot;
    if (parsed_arg_options.count("automation-data-root")) {
        if (!hasInputTape) {
            fprintf(stderr,
                    "Automation State Error: --automation-data-root requires --input-tape PATH\n");
            return 1;
        }
        automationDataRoot =
            std::filesystem::u8path(parsed_arg_options["automation-data-root"].as<std::string>());
        std::error_code dataRootError;
        if (!std::filesystem::is_directory(automationDataRoot, dataRootError)) {
            fprintf(stderr, "Automation State Error: data root '%s' is not a directory%s%s\n",
                    dusk::io::fs_path_to_string(automationDataRoot).c_str(),
                    dataRootError ? ": " : "", dataRootError ? dataRootError.message().c_str() : "");
            return 1;
        }
        automationDataRoot = std::filesystem::absolute(automationDataRoot, dataRootError);
        if (dataRootError) {
            fprintf(stderr, "Automation State Error: cannot resolve data root: %s\n",
                    dataRootError.message().c_str());
            return 1;
        }
    }
    if (parsed_arg_options.count("automation-card-root")) {
        if (!hasInputTape) {
            fprintf(stderr,
                    "Memory Card Error: --automation-card-root requires --input-tape PATH\n");
            return 1;
        }
        automationCardRoot =
            std::filesystem::u8path(parsed_arg_options["automation-card-root"].as<std::string>());
        std::error_code cardRootError;
        if (!std::filesystem::is_directory(automationCardRoot, cardRootError)) {
            fprintf(stderr, "Memory Card Error: automation card root '%s' is not a directory%s%s\n",
                    dusk::io::fs_path_to_string(automationCardRoot).c_str(),
                    cardRootError ? ": " : "", cardRootError ? cardRootError.message().c_str() : "");
            return 1;
        }
        automationCardRoot = std::filesystem::absolute(automationCardRoot, cardRootError);
        if (cardRootError) {
            fprintf(stderr, "Memory Card Error: cannot resolve automation card root: %s\n",
                    cardRootError.message().c_str());
            return 1;
        }
    }
    if (!automationDataRoot.empty() && !automationCardRoot.empty()) {
        fprintf(stderr,
                "Automation State Error: --automation-data-root and --automation-card-root are mutually exclusive\n");
        return 1;
    }

    const std::string inputTapeEnd = parsed_arg_options["input-tape-end"].as<std::string>();
    if (inputTapeEnd == "release") {
        inputTapeEndBehavior = dusk::automation::TapeEndBehavior::Release;
    } else if (inputTapeEnd == "hold") {
        inputTapeEndBehavior = dusk::automation::TapeEndBehavior::Hold;
    } else if (inputTapeEnd == "loop") {
        inputTapeEndBehavior = dusk::automation::TapeEndBehavior::Loop;
    } else {
        fprintf(stderr,
                "Input Tape Error: invalid --input-tape-end value '%s' (expected release, hold, or loop)\n",
                inputTapeEnd.c_str());
        return 1;
    }

    if (!hasInputTape && (exitAfterInputTape || inputTapeEnd != "release")) {
        fprintf(stderr, "Input Tape Error: --input-tape-end and --exit-after-tape require --input-tape PATH\n");
        return 1;
    }
    if (exitAfterInputTape && inputTapeEndBehavior == dusk::automation::TapeEndBehavior::Loop) {
        fprintf(stderr, "Input Tape Error: --exit-after-tape cannot be combined with --input-tape-end loop\n");
        return 1;
    }

    const bool hasAutomationOracle = parsed_arg_options.count("automation-oracle") != 0;
    const bool hasAutomationOracleResult =
        parsed_arg_options.count("automation-oracle-result") != 0;
    automationOracleContinueOnPass =
        parsed_arg_options["automation-oracle-continue-on-pass"].as<bool>();
    if (automationOracleContinueOnPass && !hasAutomationOracle) {
        fprintf(stderr,
                "Automation Oracle Error: --automation-oracle-continue-on-pass requires --automation-oracle NAME\n");
        return 1;
    }
    if (hasAutomationOracle != hasAutomationOracleResult) {
        fprintf(stderr,
                "Automation Oracle Error: --automation-oracle NAME and --automation-oracle-result PATH must be used together\n");
        return 1;
    }
    if (hasAutomationOracle) {
        const std::string oracleName =
            parsed_arg_options["automation-oracle"].as<std::string>();
        if (oracleName != "eye-shredder") {
            fprintf(stderr,
                    "Automation Oracle Error: unsupported oracle '%s' (expected eye-shredder)\n",
                    oracleName.c_str());
            return 1;
        }
        if (!hasInputTape) {
            fprintf(stderr,
                    "Automation Oracle Error: eye-shredder requires --input-tape PATH\n");
            return 1;
        }
        if (!cursorBreakoutShadow) {
            fprintf(stderr,
                    "Automation Oracle Error: eye-shredder requires --cursor-breakout-shadow\n");
            return 1;
        }
        if (inputTapeEndBehavior == dusk::automation::TapeEndBehavior::Loop) {
            fprintf(stderr,
                    "Automation Oracle Error: eye-shredder cannot use --input-tape-end loop\n");
            return 1;
        }
        const std::string resultPath =
            parsed_arg_options["automation-oracle-result"].as<std::string>();
        if (resultPath.empty()) {
            fprintf(stderr,
                    "Automation Oracle Error: --automation-oracle-result PATH cannot be empty\n");
            return 1;
        }
        eyeShredderOracleEnabled = true;
        eyeShredderOracleResultPath = std::filesystem::u8path(resultPath);
        eyeShredderOracle.start();
        aurora_reset_gx_channel_count_telemetry();
    }

    if (hasInputTape) {
        inputTapePath = parsed_arg_options["input-tape"].as<std::string>();

        std::vector<u8> inputTapeBytes;
        try {
            inputTapeBytes = dusk::io::FileStream::ReadAllBytes(inputTapePath.c_str());
        } catch (const std::exception& e) {
            fprintf(stderr, "Input Tape Error: cannot read '%s': %s\n", inputTapePath.c_str(), e.what());
            return 1;
        }

        dusk::automation::InputTape inputTape;
        const dusk::automation::InputTapeError tapeError =
            dusk::automation::decode_input_tape(inputTapeBytes, inputTape);
        if (tapeError != dusk::automation::InputTapeError::None) {
            fprintf(stderr, "Input Tape Error: cannot load '%s': %s\n", inputTapePath.c_str(),
                    dusk::automation::input_tape_error_message(tapeError));
            return 1;
        }
        if (inputTape.frames.empty()) {
            fprintf(stderr, "Input Tape Error: '%s' contains no input frames\n", inputTapePath.c_str());
            return 1;
        }
        if (static_cast<std::uint64_t>(inputTape.tickRateNumerator) !=
            static_cast<std::uint64_t>(inputTape.tickRateDenominator) * 30u) {
            fprintf(stderr,
                    "Input Tape Error: '%s' declares a %u/%u Hz tick rate; playback requires 30/1 Hz\n",
                    inputTapePath.c_str(), inputTape.tickRateNumerator, inputTape.tickRateDenominator);
            return 1;
        }

        inputTapeFrameCount = inputTape.frames.size();
        if (!gameplayTracePath.empty()) {
            dusk::automation::gameplay_trace_recorder().start(inputTapeFrameCount + 1);
        }
        auto& inputTapePlayer = dusk::automation::input_tape_player();
        inputTapePlayer.install(std::move(inputTape));
        if (!inputTapePlayer.start(inputTapeEndBehavior)) {
            fprintf(stderr, "Input Tape Error: failed to start '%s'\n", inputTapePath.c_str());
            return 1;
        }
    }

    const bool deterministicAutomationIo = hasInputTape && fixedStepMainLoop;
    dusk::automation::set_synchronous_io_enabled(deterministicAutomationIo);

    if (parsed_arg_options.contains("load-save")){
        uint8_t slot = parsed_arg_options["load-save"].as<uint8_t>();
        if (slot >= 1 && slot <= 3) {
            dusk::SaveRequested = slot;
        }
    }

    dusk::registerSettings();

    const auto startupLogLevel =
        static_cast<AuroraLogLevel>(parsed_arg_options["log-level"].as<uint8_t>());
    const auto dataPaths = automationDataRoot.empty()
                               ? dusk::data::initialize_data()
                               : dusk::data::initialize_automation_data(automationDataRoot);
    dusk::ConfigPath = dataPaths.userPath;
    dusk::CachePath = dataPaths.cachePath;
    dusk::InitializeFileLogging(dusk::CachePath, startupLogLevel);
    if (!automationDataRoot.empty()) {
        DuskLog.info("Automation data root: {}",
                     dusk::io::fs_path_to_string(automationDataRoot));
    }

    // Development Mode
    if (parsed_arg_options.count("develop")) {
        mDoMain::developmentMode = parsed_arg_options["develop"].as<bool>();  // Enable Dev Mode for Debugging
        dusk::OSReportReallyForceEnable = parsed_arg_options["develop"].as<bool>();  // Print OSReport to console
    }

    log_build_info();
    if (fixedStepMainLoop) {
        DuskLog.info("Automation timing: fixed 30 Hz step (headless={}, unpaced={}, initial_os_tick={})",
                     headlessMainLoop, unpacedMainLoop, deterministicInitialTicks);
        DuskLog.warn("Deterministic OS time does not dispatch OSAlarm callbacks; pre-loop time "
                     "remains fixed at the declared initial tick until the first completed simulation tick");
    }
    if (headlessMainLoop) {
        DuskLog.info("Headless audio: host output muted; audio emulation remains active");
    }
    if (hasNameEntryTrace) {
        DuskLog.info("Name-entry trace: {} (fidelity={})",
                     dusk::io::fs_path_to_string(nameEntryTracePath),
                     cursorBreakoutShadow ? "cursor_breakout_shadow" : "observe_only");
    }
    if (eyeShredderOracleEnabled) {
        DuskLog.info("Automation oracle: eye-shredder -> {} (continue_on_pass={})",
                     dusk::io::fs_path_to_string(eyeShredderOracleResultPath),
                     automationOracleContinueOnPass);
    }
    if (hasInputTape) {
        DuskLog.info("Input tape: {} ({} frames, end={}, exit={})", inputTapePath,
                     inputTapeFrameCount, inputTapeEnd, exitAfterInputTape);
    }

    dusk::config::load_from_user_preferences();
    ApplyCVarOverrides(parsed_arg_options["cvar"]);
    dusk::android::update_surface_frame_rate();
    dusk::crash_reporting::initialize();
    dusk::crash_handler::install();
    // TODO: How to handle this?
    // PADSetDefaultMapping(&defaultPadMapping, PAD_TYPE_STANDARD);

    {
        const auto mappingsPath = dusk::ConfigPath / "gamecontrollerdb.txt";
        std::error_code ec;
        if (std::filesystem::exists(mappingsPath, ec)) {
            const auto mappingsPathString = dusk::io::fs_path_to_string(mappingsPath);
            if (SDL_AddGamepadMappingsFromFile(mappingsPathString.c_str()) < 0) {
                DuskLog.warn("Failed to load gamecontrollerdb.txt from '{}': {}",
                    mappingsPathString, SDL_GetError());
            }
        } else if (ec) {
            DuskLog.warn("Failed to inspect gamecontrollerdb.txt in data folder '{}': {}",
                dusk::io::fs_path_to_string(mappingsPath), ec.message());
        }
    }

    // Set SDL metadata for audio mixers and macOS "About" menu
    SDL_SetAppMetadata("Dusklight", DUSK_VERSION_STRING, "dev.twilitrealm.dusk");

    {
        const auto userPathString = dusk::ConfigPath.u8string();
        const auto cachePathString = dusk::CachePath.u8string();
        AuroraConfig config{};
        config.appName = dusk::AppName;
        config.userPath = reinterpret_cast<const char*>(userPathString.c_str());
        config.cachePath = reinterpret_cast<const char*>(cachePathString.c_str());
#ifdef DUSK_ASSET_DIR
        config.resourcesPath = DUSK_ASSET_DIR;
#endif
        config.vsync = fixedStepMainLoop ? false : dusk::getSettings().video.enableVsync;
        config.startFullscreen = headlessMainLoop ? false : dusk::getSettings().video.enableFullscreen;
        config.windowPosX = -1;
        config.windowPosY = -1;

        const int lastWindowWidth = dusk::getSettings().video.lastWindowWidth.getValue();
        const int lastWindowHeight = dusk::getSettings().video.lastWindowHeight.getValue();

        if (dusk::getSettings().video.rememberWindowSize && lastWindowWidth > 0 && lastWindowHeight > 0) {
            config.windowWidth = lastWindowWidth;
            config.windowHeight = lastWindowHeight;
        } else {
            config.windowWidth = defaultWindowWidth * 2;
            config.windowHeight = defaultWindowHeight * 2;
        }

        config.desiredBackend = headlessMainLoop ? BACKEND_NULL : ResolveDesiredBackend(parsed_arg_options);
        config.logCallback = &aurora_log_callback;
        config.logLevel = startupLogLevel;
        config.mem1Size = 256 * 1024 * 1024;
        config.mem2Size = 24 * 1024 * 1024;
        config.allowJoystickBackgroundEvents = dusk::getSettings().game.allowBackgroundInput;
        config.pauseOnFocusLost = headlessMainLoop ? false : dusk::getSettings().game.pauseOnFocusLost;
        config.imGuiInitCallback = &aurora_imgui_init_callback;
        config.allowTextureDumps = false;
        config.disablePresentation = headlessMainLoop;
        auroraInfo = aurora_initialize(argc, argv, &config);
    }

    aurora_dvd_set_synchronous(deterministicAutomationIo);
    if (deterministicAutomationIo) {
        DuskLog.info("Automation I/O: DVD and memory-card commands complete on the simulation thread");
    }

    automationInputQuarantine = hasInputTape;
    aurora_set_automation_input_quarantine(automationInputQuarantine);
    if (automationInputQuarantine) {
        DuskLog.info("Automation input quarantine enabled; host keyboard, mouse, touch, gamepad UI, "
                     "mouse camera, and gyro input are suppressed until process exit");
    }

    if (headlessMainLoop) {
        if (auroraInfo.backend != BACKEND_NULL) {
            DuskLog.error("Headless mode requested the null backend, but Aurora selected {}",
                          dusk::backend_name(auroraInfo.backend));
            dusk::crash_reporting::shutdown();
            dusk::ShutdownFileLogging();
            dusk::config::shutdown();
            aurora_shutdown();
            return 1;
        }
        // Aurora retains a hidden SDL window as an internal size/event anchor,
        // but simulation-only mode must never expose it on the desktop or taskbar.
        // disablePresentation bypasses Aurora's visibility/focus pause checks.
        if ((SDL_GetWindowFlags(auroraInfo.window) & SDL_WINDOW_HIDDEN) == 0u) {
            DuskLog.error("Headless Aurora window unexpectedly became visible");
            dusk::crash_reporting::shutdown();
            dusk::ShutdownFileLogging();
            dusk::config::shutdown();
            aurora_shutdown();
            return 1;
        }
    }

    // Apply after aurora_initialize: speedrun mode mutates cvars whose change callbacks push
    // values into aurora.
    if (dusk::getSettings().game.speedrunMode) {
        dusk::resetForSpeedrunMode();
    }

#ifdef DUSK_DISCORD
    if (!headlessMainLoop && dusk::getSettings().game.enableDiscordPresence) {
        dusk::discord::initialize();
    }
#endif

    VISetWindowTitle(
        fmt::format("Dusklight {} [{}]", DUSK_WC_DESCRIBE, dusk::backend_name(auroraInfo.backend))
        .c_str());

    if (dusk::getSettings().video.lockAspectRatio) {
        AuroraSetViewportPolicy(AURORA_VIEWPORT_FIT);
    } else {
        AuroraSetViewportPolicy(AURORA_VIEWPORT_STRETCH);
    }
    VISetFrameBufferScale(dusk::getSettings().game.internalResolutionScale.getValue());
    switch (dusk::getSettings().game.resampler.getValue()) {
    case dusk::Resampler::Area:
        aurora_set_resampler(SAMPLER_AREA);
        break;
    case dusk::Resampler::Bilinear:
    default:
        aurora_set_resampler(SAMPLER_BILINEAR);
        break;
    }

    dusk::audio::SetOutputMuted(headlessMainLoop);
    dusk::audio::SetMasterVolume(dusk::audio::MasterVolumeToLinear(dusk::getSettings().audio.masterVolume / 100.0f));
    dusk::audio::SetEnableReverb(dusk::getSettings().audio.enableReverb);
    dusk::audio::EnableHrtf = dusk::getSettings().audio.enableHrtf;

    // Run ImGui UI loop if Aurora couldn't initialize a backend
    if (auroraInfo.backend == BACKEND_NULL && !headlessMainLoop) {
        launchUILoop();
        dusk::crash_reporting::shutdown();
        dusk::ShutdownFileLogging();
        fflush(stdout);
        fflush(stderr);
#ifdef DUSK_DISCORD
        dusk::discord::shutdown();
#endif
        dusk::ui::shutdown();
        aurora_shutdown();
        return 0;
    }

    dusk::texture_replacements::reload();
    dusk::ui::initialize();
    dusk::ui::push_document(std::make_unique<dusk::ui::Overlay>(), true, true);
    dusk::ui::push_document(std::make_unique<dusk::ui::TouchControls>(), false, true);
    dusk::ui::push_document(std::make_unique<dusk::ui::MenuBar>(), false);

    // Invalidate a bad saved isoPath so that Dusklight can't get blocked from starting up.
    // This is only a metadata check; full hash verification is handled by the prelaunch UI.
    bool forcePreLaunchUI = false;
    bool saveConfigBeforePrelaunch = false;

    const std::string p = dusk::getSettings().backend.isoPath;
    dusk::iso::DiscInfo discInfo{};
    if (!p.empty() &&
        dusk::iso::inspect(p.c_str(), discInfo) != dusk::iso::ValidationError::Success)
    {
        DuskLog.warn("Saved DVD image path failed validation, clearing configured path: {}", p);
        dusk::getSettings().backend.isoPath.setValue("");
        dusk::getSettings().backend.isoVerification.setValue(dusk::DiscVerificationState::Unknown);
        forcePreLaunchUI = true;
        saveConfigBeforePrelaunch = true;
    }

    std::string dvd_path = dusk::getSettings().backend.isoPath;
    bool dvd_opened = false;
    if (parsed_arg_options.count("dvd") || useConfiguredDvd) {
        if (parsed_arg_options.count("dvd")) {
            dvd_path = parsed_arg_options["dvd"].as<std::string>();
        }
        if (dusk::iso::inspect(dvd_path.c_str(), discInfo) == dusk::iso::ValidationError::Success) {
            DuskLog.info("Loading DVD image {}: {}",
                         useConfiguredDvd ? "from configured path" : "from command line", dvd_path);
            dvd_opened = aurora_dvd_open(dvd_path.c_str());
            if (!dvd_opened) {
                if (headlessMainLoop) {
                    DuskLog.warn("Failed to open DVD image from command line in headless mode: {}", dvd_path);
                } else {
                    DuskLog.warn("Failed to open DVD image from command line: {}, opening prelaunch UI", dvd_path);
                }
                forcePreLaunchUI = true;
            } else {
                dusk::getSettings().backend.isoPath.setValue(dvd_path);
                dusk::getSettings().backend.isoVerification.setValue(
                    dusk::DiscVerificationState::Unknown);
                dusk::config::save();
                dusk::IsGameLaunched = true;
            }
        } else {
            if (headlessMainLoop) {
                DuskLog.warn("DVD image from command line failed validation in headless mode: {}", dvd_path);
            } else {
                DuskLog.warn("DVD image from command line failed validation: {}, opening prelaunch UI", dvd_path);
            }
            forcePreLaunchUI = true;
        }
    }

    if ((headlessMainLoop || useConfiguredDvd || hasInputTape) && !dvd_opened) {
        DuskLog.error("{} could not validate and open the requested DVD image: {}",
                      headlessMainLoop ? "Headless mode" : "Configured DVD boot", dvd_path);
        dusk::crash_reporting::shutdown();
        dusk::ShutdownFileLogging();
#ifdef DUSK_DISCORD
        dusk::discord::shutdown();
#endif
        dusk::ui::shutdown();
        dusk::texture_replacements::shutdown();
        dusk::config::shutdown();
        aurora_shutdown();
        return 1;
    }

    bool skipPreLaunchUI = dusk::getSettings().backend.skipPreLaunchUI.getValue();

    // If we can't load right into the game, stop requesting to load a stage or save
    if (forcePreLaunchUI || dvd_path.empty()) {
        if (dusk::StageRequested.set) {
            DuskLog.warn("Cannot load stage {} because no iso path is set, opening prelaunch UI",dusk::StageRequested.stage);
            dusk::StageRequested = {};
        }
        if (dusk::SaveRequested) {
            DuskLog.warn("Cannot load save {} because no iso path is set, opening prelaunch UI",dusk::SaveRequested);
            dusk::SaveRequested = 0;
        }
    }else if (dusk::StageRequested.set || dusk::SaveRequested) {
        skipPreLaunchUI = true;
    }

    dusk::iso::log_verification_state(
        dusk::getSettings().backend.isoPath.getValue(),
        dusk::getSettings().backend.isoVerification.getValue());

    if (!dvd_opened) {
        if (dusk::getSettings().backend.isoPath.getValue().empty()) {
            forcePreLaunchUI = true;
        }
        if (forcePreLaunchUI && skipPreLaunchUI) {
            DuskLog.warn("Prelaunch UI was disabled with no usable DVD image, enabling prelaunch UI");
            dusk::getSettings().backend.skipPreLaunchUI.setValue(false);
            saveConfigBeforePrelaunch = true;
        }
        if (saveConfigBeforePrelaunch) {
            dusk::config::save();
        }

        if (!skipPreLaunchUI) {
            dusk::ui::push_document(std::make_unique<dusk::ui::Prelaunch>(), true);

            // pre game launch ui main loop
            if (!launchUILoop()) {
                dusk::crash_reporting::shutdown();
                dusk::ShutdownFileLogging();
                fflush(stdout);
                fflush(stderr);
#ifdef DUSK_DISCORD
                dusk::discord::shutdown();
#endif
                dusk::ui::shutdown();
                aurora_shutdown();
                return 0;
            }
        }

        dvd_path = dusk::getSettings().backend.isoPath;
        if (dvd_path.empty()) {
            DuskLog.fatal("No DVD image specified, unable to boot!");
        }
        if (!dusk::IsGameLaunched &&
            dusk::iso::inspect(dvd_path.c_str(), discInfo) != dusk::iso::ValidationError::Success)
        {
            DuskLog.fatal("DVD image failed validation: {}", dvd_path);
        }
        DuskLog.info("Loading DVD image: {}", dvd_path);
        if (!aurora_dvd_open(dvd_path.c_str())) {
            DuskLog.fatal("Failed to open DVD image: {}", dvd_path);
        }

        dusk::IsGameLaunched = true;
    }

    if (!automationCardRoot.empty()) {
        CARDSetLoadType(
            static_cast<CARDFileType>(dusk::getSettings().backend.cardFileType.getValue()));
        const auto cardRootUtf8 = automationCardRoot.u8string();
        CARDSetBasePath(reinterpret_cast<const char*>(cardRootUtf8.c_str()), -1);
        DuskLog.info("Automation memory-card root: {}",
                     dusk::io::fs_path_to_string(automationCardRoot));
    }

#if DUSK_ENABLE_SENTRY_NATIVE
    if (dusk::crash_reporting::get_consent() == dusk::crash_reporting::Consent::Unknown) {
        dusk::ui::push_document(std::make_unique<dusk::ui::CrashReportWindow>());
    }
#endif

    if (!dusk::getSettings().backend.wasPresetChosen) {
        dusk::ui::push_document(std::make_unique<dusk::ui::PresetWindow>());
    }

    dusk::version::init();
    if (eyeShredderOracleEnabled &&
        dusk::version::getGameVersion() != dusk::version::GameVersion::GcnUsa) {
        const std::string reason =
            "Eye Shredder oracle requires a GameCube USA disc; its expected address and bytes are specific to a fresh NTSC-U file";
        DuskLog.error("{}", reason);
        eyeShredderOracle.reject(reason);
        dusk::IsRunning = false;
    }
    LanguageInit();

    OSInit();

    mDoMain::sPowerOnTime = OSGetTime();

    // Reset Data
    static mDoRstData sResetData = {0};
    mDoRst::setResetData(&sResetData);
    mDoRst::offReset();
    mDoRst::setLogoScnFlag(0);

    // Global Context Init
    dComIfG_ct();

    mDoDvdThd::SyncWidthSound = false;

    // Mod search directories, highest priority first: user dir (--mods replaces it), then
    // mods/ next to the app, then install-bundled mods inside the app bundle.
    {
        std::vector<dusk::mods::ModSearchDir> modDirs;
        if (parsed_arg_options.contains("mods") &&
            !parsed_arg_options["mods"].as<std::string>().empty())
        {
            modDirs.push_back({.path = parsed_arg_options["mods"].as<std::string>()});
        } else {
            modDirs.push_back({.path = dusk::ConfigPath / "mods"});
        }
#if TARGET_ANDROID
        // APK-bundled mods are extracted to internal storage
        // by DuskActivity before SDL_main runs.
        modDirs.push_back({
            .path = dusk::CachePath / "bundled_mods",
        });
#elif defined(__APPLE__) && (TARGET_OS_IOS || TARGET_OS_TV)
        modDirs.push_back({
            .path = dusk::data::base_path_relative("mods"),
            .inPlaceNative = true,
            .nativeLibDir = dusk::data::base_path_relative("Frameworks"),
        });
#else
#if defined(__APPLE__)
        // Base path is Contents/Resources; search up for dev mods
        // TODO: scope to non-CI builds
        modDirs.push_back({
            .path = dusk::data::base_path_relative("../../../mods").lexically_normal(),
            .inPlaceNative = true,
        });
        // Contents/Resources/mods
        modDirs.push_back({
            .path = dusk::data::base_path_relative("mods"),
            .inPlaceNative = true,
        });
#else
        modDirs.push_back({
            .path = dusk::data::base_path_relative("mods"),
            .inPlaceNative = true,
        });
#endif
#endif
        dusk::mods::ModLoader::instance().set_search_dirs(std::move(modDirs));
    }
#if TARGET_ANDROID
    // A user-relocated data dir can live on external storage, which is mounted noexec.
    // Native mod libraries must be extracted to internal storage.
    dusk::mods::ModLoader::instance().set_cache_dir(dusk::CachePath / "mod_cache");
#endif

    DuskLog.info("Initializing mods...");
    dusk::mods::ModLoader::instance().init();

    OSReport("Starting main01 (Game Loop)...\n");

    main01();

    dusk::MoviePlayerShutdown();

    dusk::crash_reporting::shutdown();
    dusk::ShutdownFileLogging();
    fflush(stdout);
    fflush(stderr);

    mDoMch_Destroy();

    // Notifies all CVs and causes threads to exit
    OSResetSystem(OS_RESET_SHUTDOWN, 0, 0);

#ifdef DUSK_DISCORD
    dusk::discord::shutdown();
#endif
    dusk::ui::shutdown();
    dusk::texture_replacements::shutdown();
    dusk::config::shutdown();
    aurora_shutdown();

    if (deterministicTimeEnabled) {
        AuroraDisableDeterministicTime();
        deterministicTimeEnabled = false;
    }

    const bool eyeShredderOracleFailed =
        eyeShredderOracleEnabled &&
        eyeShredderOracle.result().status !=
            dusk::automation::EyeShredderOracleStatus::Passed;
    const auto& milestoneTracker = dusk::automation::milestone_tracker();
    const bool milestoneGoalFailed =
        milestoneTracker.goal().has_value() && !milestoneTracker.goalReached();
    return nameEntryTraceWriteFailed || gameplayTraceWriteFailed || milestoneResultWriteFailed ||
                   milestoneGoalFailed ||
                   eyeShredderOracleResultWriteFailed ||
                   eyeShredderOracleFailed || deterministicTimeAdvanceFailed ||
                   inputTapePlaybackFailed
               ? 1
               : 0;
}


bool JKRHeap::dump_sort() {
    return true;
}

#ifdef __MWERKS__
template <typename T>
JHIComPortManager<T>* JHIComPortManager<T>::instance = nullptr;

template <>
JHIComPortManager<JHICmnMem>* JHIComPortManager<JHICmnMem>::instance = nullptr;

template<>
Z2WolfHowlMgr* JASGlobalInstance<Z2WolfHowlMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2EnvSeMgr* JASGlobalInstance<Z2EnvSeMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2FxLineMgr* JASGlobalInstance<Z2FxLineMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2Audience* JASGlobalInstance<Z2Audience>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SoundObjMgr* JASGlobalInstance<Z2SoundObjMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SoundInfo* JASGlobalInstance<Z2SoundInfo>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAUSoundInfo* JASGlobalInstance<JAUSoundInfo>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAUSoundNameTable* JASGlobalInstance<JAUSoundNameTable>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAUSoundTable* JASGlobalInstance<JAUSoundTable>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAISoundInfo* JASGlobalInstance<JAISoundInfo>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SoundMgr* JASGlobalInstance<Z2SoundMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAIStreamMgr* JASGlobalInstance<JAIStreamMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAISeqMgr* JASGlobalInstance<JAISeqMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAISeMgr* JASGlobalInstance<JAISeMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SpeechMgr2* JASGlobalInstance<Z2SpeechMgr2>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SoundStarter* JASGlobalInstance<Z2SoundStarter>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JAISoundStarter* JASGlobalInstance<JAISoundStarter>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2StatusMgr* JASGlobalInstance<Z2StatusMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SceneMgr* JASGlobalInstance<Z2SceneMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SeqMgr* JASGlobalInstance<Z2SeqMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
Z2SeMgr* JASGlobalInstance<Z2SeMgr>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JASAudioThread* JASGlobalInstance<JASAudioThread>::sInstance JAS_GLOBAL_INSTANCE_INIT;

template<>
JASDefaultBankTable* JASGlobalInstance<JASDefaultBankTable>::sInstance JAS_GLOBAL_INSTANCE_INIT;
#endif // __MWERKS__
