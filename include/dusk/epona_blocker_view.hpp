#ifndef DUSK_EPONA_BLOCKER_VIEW_HPP
#define DUSK_EPONA_BLOCKER_VIEW_HPP

namespace dusk {

// Enqueues read-only debug geometry for the collision metadata and actor
// volumes that explicitly prevent Epona from entering an area.
void draw_epona_blocker_view();

}  // namespace dusk

#endif  // DUSK_EPONA_BLOCKER_VIEW_HPP
