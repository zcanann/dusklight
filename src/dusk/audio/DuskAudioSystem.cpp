#include "dusk/audio/DuskAudioSystem.h"

#include "dusk/automation/io_mode.hpp"

#include <SDL3/SDL_init.h>
#include <array>
#include <cassert>
#include <cstdint>
#include <span>

#include "JSystem/JAudio2/JASAiCtrl.h"
#include "JSystem/JAudio2/JASChannel.h"
#include "JSystem/JAudio2/JASCriticalSection.h"
#include "JSystem/JAudio2/JASDSPChannel.h"
#include "JSystem/JAudio2/JASHeapCtrl.h"

#include "DuskDsp.hpp"
#include "JSystem/JAudio2/JASAudioThread.h"
#include "JSystem/JAudio2/JASDriverIF.h"
#include "tracy/Tracy.hpp"

#include "dusk/automation/native_state_section_begin.h"

using namespace dusk::audio;

static OutputSubframe OutBuffer;
static std::array<f32, DSP_SUBFRAME_SIZE * OutputSubframe::NUM_CHANNELS> OutInterleaveBuffer;

static SDL_AudioStream* PlaybackStream;
static bool OutputMuted;
static std::uint64_t DeterministicSampleCredit;

/**
 * SDL audiostream callback to trigger rendering of new audio data.
 */
static void SDLCALL GetNewAudio(
    void*,
    SDL_AudioStream*,
    int needed,
    int);

/**
 * Render an entire new frame of audio and output it to SDL3.
 * Note: "audio frames" are unrelated to video frames.
 * @return Amount of audio samples rendered.
 */
static int RenderNewAudioFrame(bool submitToHost);

/**
 * Render an audio subframe and output it to SDL3.
 */
static void RenderAudioSubframe(bool submitToHost);

static void InitSDL3Output() {
    SDL_Init(SDL_INIT_AUDIO);

    constexpr SDL_AudioSpec spec = {
        SDL_AUDIO_F32,
        2,
        SampleRate,
    };
    PlaybackStream = SDL_OpenAudioDeviceStream(
        SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
        &spec,
        &GetNewAudio,
        nullptr);
}

void dusk::audio::Initialize() {
    InitSDL3Output();
    SDL_SetAudioStreamGain(PlaybackStream, OutputMuted ? 0.0f : 1.0f);
    DspInit();

    JASDsp::initBuffer();
    JASDSPChannel::initAll();

    JASPoolAllocObject_MultiThreaded<JASChannel>::newMemPool(0x48);
    if (!dusk::automation::synchronous_io_enabled() && PlaybackStream != nullptr) {
        SDL_ResumeAudioStreamDevice(PlaybackStream);
    }
}

void dusk::audio::SetMasterVolume(const f32 value) {
    JASCriticalSection section;

    MasterVolume = value;
}

void dusk::audio::SetOutputMuted(const bool muted) {
    OutputMuted = muted;
    if (PlaybackStream != nullptr) {
        SDL_SetAudioStreamGain(PlaybackStream, muted ? 0.0f : 1.0f);
    }
}

void dusk::audio::SetPaused(const bool paused) {
    if (PlaybackStream == nullptr) {
        return;
    }
    if (paused) {
        SDL_PauseAudioStreamDevice(PlaybackStream);
    } else {
        SDL_ResumeAudioStreamDevice(PlaybackStream);
    }
}

void dusk::audio::AdvanceDeterministicAutomationTick() {
    if (!dusk::automation::synchronous_io_enabled() || PlaybackStream == nullptr) {
        return;
    }

    const std::uint64_t audioFrameSamples =
        static_cast<std::uint64_t>(JASDriver::getSubFrames()) * DSP_SUBFRAME_SIZE;
    if (audioFrameSamples == 0) {
        return;
    }

    // A simulation tick is exactly 1/30 second. Keep the remainder in integer
    // sample-rate units so the render cadence is deterministic and drift-free.
    DeterministicSampleCredit += SampleRate;
    const std::uint64_t audioFrameCost = 30 * audioFrameSamples;
    while (DeterministicSampleCredit >= audioFrameCost) {
        RenderNewAudioFrame(false);
        DeterministicSampleCredit -= audioFrameCost;
    }
}

bool dusk::audio::QuiesceForStateCheckpoint() {
    JASCriticalSection section;
    if (PlaybackStream == nullptr || dusk::automation::synchronous_io_enabled()) {
        return true;
    }
    return SDL_PauseAudioStreamDevice(PlaybackStream);
}

void dusk::audio::SetEnableReverb(const bool value) {
    JASCriticalSection section;

    EnableReverb = value;
}

#ifdef TRACY_ENABLE
static auto FrameName = "GetNewAudio";
#endif

void SDLCALL GetNewAudio(
    void*,
    SDL_AudioStream*,
    int needed,
    int) {
    FrameMarkStart(FrameName);
    while (needed > 0) {
        const int rendered = RenderNewAudioFrame(true);
        needed -= rendered;
    }
    FrameMarkEnd(FrameName);
}

int RenderNewAudioFrame(const bool submitToHost) {
    ZoneScoped;
    JASCriticalSection section;
    const u32 countSubframes = JASDriver::getSubFrames();

    JASAudioThread::setDSPSyncCount(countSubframes);

    for (u32 i = 0; i < countSubframes; i++) {
        RenderAudioSubframe(submitToHost);

        JASAudioThread::snIntCount -= 1;
    }

    return static_cast<u16>(countSubframes) * DSP_SUBFRAME_SIZE;
}

static void InterleaveOutputData(const OutputSubframe& data, std::span<f32> target) {
    assert(target.size() >= data.channels[0].size() * OutputSubframe::NUM_CHANNELS);

    size_t outPos = 0;
    for (size_t inPos = 0; inPos < data.channels[0].size(); inPos++) {
        for (size_t channelIdx = 0; channelIdx < OutputSubframe::NUM_CHANNELS; channelIdx++) {
            target[outPos++] = data.channels[channelIdx][inPos];
        }
    }
}

void RenderAudioSubframe(const bool submitToHost) {
    ZoneScoped;
    OutBuffer = {};

    JASDriver::updateDSP();
    DspRender(OutBuffer);

    InterleaveOutputData(OutBuffer, OutInterleaveBuffer);

    if (JASDriver::extMixCallback != nullptr && JASDriver::sMixMode == MIX_MODE_INTERLEAVE) {
        static_assert(OutputSubframe::NUM_CHANNELS == 2); // This code only works with Stereo so far.
        // NOTE: In the real game, this gets called on the entire audio frame, rather than the subframe.
        // That's probably more efficient, but I didn't wanna change the code to calculate the
        // entire audio buffers at once.
        // This is only used for the movie player, and it seems to work fine with the smaller calls.
        const auto mixData = JASDriver::extMixCallback(DSP_SUBFRAME_SIZE);
        if (mixData) {
            for (int i = 0; i < OutInterleaveBuffer.size(); i++) {
                OutInterleaveBuffer[i] += static_cast<f32>(mixData[i]) / static_cast<f32>(0x7FFF);
            }
        }
    }

    if (submitToHost && PlaybackStream != nullptr) {
        SDL_PutAudioStreamData(PlaybackStream, &OutInterleaveBuffer,
            sizeof(OutInterleaveBuffer));
    }
}

u32 dusk::audio::GetResetCount(int channelIdx) {
    return ChannelAux[channelIdx].resetCount;
}

f32 dusk::audio::VolumeFromU16(u16 value) {
    return static_cast<f32>(value) / static_cast<f32>(JASDriver::getChannelLevel_dsp());
}
