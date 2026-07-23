# EMS layered feasibility

The EMS validation profile in `crates/engine/src/ems.rs` is an exact-context
solver fixture for the route from the Sacred Grove to the Master Sword, Hyrule
Castle, and Ganon. It deliberately separates logical authorization from
physical feasibility.

## Upper bound

Upper-bound search keeps hard state guards and technique prerequisites but
relaxes physical obligations and obstructions. From the fixture's wolf-form,
Faron-twilight start, it can therefore produce the logical sequence:

1. acquire the Master Sword;
2. enter Hyrule Castle by an authorized approach;
3. defeat Ganon.

This is a superset proof, not a claim that every physical step works.

## Modeled obstructions

Modeled search enables independently scoped constraints for:

- human form during the sword setup;
- Faron twilight on the standard castle approach;
- Epona mount and non-twilight state on the Epona approach;
- the standard and Epona collision boundaries; and
- the physical execution of the Epona OOB setup.

The standard EMS technique performs the modeled wolf-to-human setup and
discharges only the charge-attack and standard-boundary obligations. It does
not erase the distinct twilight obstruction. Consequently the permissive path
is removed when the obstruction catalog is enabled.

## Epona and rupee alternative

From a controlled, human, Epona-mounted, non-twilight state, modeled search can
compose the alternate route. Rupee clip discharges only the charge-attack
approach, Epona OOB discharges only its execution obligation, and the scoped
boundary resolver bypasses only the Epona castle obstruction. The mount and
non-twilight predicates remain independently checked.

The tests demonstrate all three claims: upper-bound reachability, refinement
after obstruction knowledge is enabled, and successful composition of the
Epona/rupee route. Removing obstructions for the refinement comparison also
removes their resolvers; no dangling resolver is treated as permission.
