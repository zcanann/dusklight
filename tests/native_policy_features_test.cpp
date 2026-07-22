#include "dusk/automation/native_policy_features.hpp"

#include <cmath>
#include <fstream>
#include <iostream>
#include <iterator>
#include <limits>
#include <string>

namespace {

int failures = 0;

#define CHECK(condition, message) check((condition), (message), __LINE__)

void check(const bool condition, const char* message, const int line) {
    if (condition) return;
    std::cerr << "native_policy_features_test.cpp:" << line << ": " << message << '\n';
    ++failures;
}

bool close(const float left, const float right) {
    return std::abs(left - right) <= 0.000001F;
}

}  // namespace

int main() {
    using namespace dusk::automation;
    std::ifstream schemaStream(DUSK_NATIVE_POLICY_FEATURE_SCHEMA_PATH, std::ios::binary);
    std::string schema((std::istreambuf_iterator<char>(schemaStream)), {});
    for (std::size_t offset = schema.find("\r\n"); offset != std::string::npos;
         offset = schema.find("\r\n"))
        schema.erase(offset, 1);
    CHECK(schema == kNativePolicyFeatureSchema, "shared schema text differs from native contract");
    CHECK(kNativePolicyFeatureSchemaSha256 ==
            "b0b708c7594f25f74e3f82c8ebbde2b737cc4f1f0076e114efddb823a538d147",
        "schema digest differs");

    NativePolicyFeatureInput input;
    input.playerPresent = true;
    input.playerIsLink = true;
    input.playerPosition = {1.5F, -2.0F, 3.25F};
    input.playerVelocity = {-4.0F, 5.0F, -6.0F};
    input.playerForwardSpeed = 7.0F;
    input.playerCurrentYaw = -16384;
    input.playerShapeYaw = 8192;
    input.playerGroundContact = true;
    input.playerRoofContact = true;
    input.playerWaterIn = true;
    input.playerGroundHeightPresent = true;
    input.playerGroundHeight = -12.5F;
    input.playerRoofHeightPresent = true;
    input.playerRoofHeight = 35.0F;
    input.eventRunning = true;
    input.eventMode = 128;
    input.eventStatus = 64;
    input.eventMapToolId = 255;
    input.nextStageEnabled = true;
    input.cameraPresent = true;
    input.cameraYawRadians = -1.25F;
    input.collisionCorrectionPresent = true;
    input.collisionCorrectionX = 0.5F;
    input.collisionCorrectionZ = -0.75F;
    input.remainingTicks = 123;
    input.previousInput.buttons = 0x8001;
    input.previousInput.stickX = -128;
    input.previousInput.stickY = 127;
    input.previousInput.substickX = -64;
    input.previousInput.substickY = 63;
    input.previousInput.triggerLeft = 255;
    input.previousInput.triggerRight = 128;
    input.previousInput.analogA = 64;
    input.previousInput.analogB = 0;
    input.previousInput.flags = RawPadFlags::Connected;
    input.previousInput.error = -64;
    input.playerDamageWaitTimer = -16384;
    input.playerIceDamageWaitTimer = 8192;
    input.playerSwordChangeWaitTimer = 128;
    input.playerDoStatus = 255;
    input.stageName = {'F', '_', 'S', 'P', '1', '0', '3', 0};
    input.room = -64;
    input.layer = 32;
    input.point = -16384;
    input.playerProcedure = 0x8001;
    input.playerModeFlags = 0x80000001;

    NativePolicyFeatureRow row{};
    std::string error;
    CHECK(encode_native_policy_features(input, row, error), "representative row did not encode");
    CHECK(error.empty(), "successful encode retained an error");
    CHECK(row.size() == 120, "feature width differs");
    CHECK(row[0] == 1.0F && row[1] == 1.0F, "player masks differ");
    CHECK(row[2] == 1.5F && row[7] == -6.0F, "player vector differs");
    CHECK(row[9] == -0.5F && row[10] == 0.25F, "yaw normalization differs");
    CHECK(row[11] == 1.0F && row[12] == 0.0F && row[15] == 1.0F,
        "contact bits differ");
    CHECK(row[16] == 1.0F && row[17] == -12.5F && row[19] == 35.0F,
        "height masks differ");
    CHECK(close(row[21], 128.0F / 255.0F) && row[23] == 1.0F,
        "event normalization differs");
    CHECK(row[25] == 1.0F && row[26] == -1.25F, "camera features differ");
    CHECK(row[27] == 1.0F && row[28] == 0.5F && row[29] == -0.75F,
        "collision correction differs");
    CHECK(row[30] == 123.0F && row[31] == 1.0F, "horizon or PAD mask differs");
    CHECK(row[32] == -1.0F && row[33] == 1.0F && row[34] == -0.5F,
        "PAD axes differ");
    CHECK(row[40] == 1.0F && row[55] == 1.0F, "PAD buttons differ");
    CHECK(row[56] == -0.5F && row[57] == -0.5F && row[58] == 0.25F,
        "timer or error normalization differs");
    CHECK(close(row[61], static_cast<float>('F') / 127.0F) && row[68] == 0.0F,
        "stage bytes differ");
    CHECK(row[69] == -0.5F && row[70] == 0.25F && row[71] == -0.5F,
        "location normalization differs");
    CHECK(row[72] == 1.0F && row[87] == 1.0F, "procedure bits differ");
    CHECK(row[88] == 1.0F && row[119] == 1.0F, "mode bits differ");

    NativePolicyFeatureInput missing = input;
    missing.playerPresent = false;
    CHECK(encode_native_policy_features(missing, row, error), "missing-player row failed");
    CHECK(row[1] == 0.0F && row[2] == 0.0F && row[16] == 0.0F && row[72] == 0.0F &&
            row[119] == 0.0F,
        "missing player did not mask dependent values");

    NativePolicyFeatureInput invalid = input;
    invalid.cameraYawRadians = std::numeric_limits<float>::infinity();
    CHECK(!encode_native_policy_features(invalid, row, error), "non-finite row was accepted");
    invalid = input;
    invalid.stageName[0] = static_cast<char>(0xff);
    CHECK(!encode_native_policy_features(invalid, row, error), "non-ASCII stage was accepted");
    return failures == 0 ? 0 : 1;
}
