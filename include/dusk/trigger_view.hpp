#ifndef DUSK_TRIGGER_VIEW_HPP
#define DUSK_TRIGGER_VIEW_HPP

namespace dusk {

// Enqueues read-only debug geometry for the current rendered frame. This does
// not participate in simulation, trigger evaluation, or automation evidence.
void draw_trigger_view();

}  // namespace dusk

#endif  // DUSK_TRIGGER_VIEW_HPP
