#pragma once

namespace dusk::automation {

// Game-thread state exposed to semantic input-tape gates. A gate observes the
// previous completed game tick, then supplies input to the same handler on the
// next tick.
class FileSelectObserver {
public:
    void setNoSavePromptReady(bool ready) { mNoSavePromptReady = ready; }
    bool noSavePromptReady() const { return mNoSavePromptReady; }
    void setDataSelectReady(bool ready) { mDataSelectReady = ready; }
    bool dataSelectReady() const { return mDataSelectReady; }
    void setKeyWaitReady(bool ready) { mKeyWaitReady = ready; }
    bool keyWaitReady() const { return mKeyWaitReady; }
    void setYesNoSelectReady(bool ready) { mYesNoSelectReady = ready; }
    bool yesNoSelectReady() const { return mYesNoSelectReady; }
    bool acceptReady() const {
        return mNoSavePromptReady || mDataSelectReady || mKeyWaitReady || mYesNoSelectReady;
    }

private:
    bool mNoSavePromptReady = false;
    bool mDataSelectReady = false;
    bool mKeyWaitReady = false;
    bool mYesNoSelectReady = false;
};

inline FileSelectObserver& file_select_observer() {
    static FileSelectObserver observer;
    return observer;
}

}  // namespace dusk::automation
