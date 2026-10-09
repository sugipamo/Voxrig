#!/usr/bin/env python3
"""Bubble-column contacts and boats in source/flowing/submerged water."""
import json
import sys
from pathlib import Path

from boat_scenarios import scenarios as ordinary_boats
from scenarios import scenario, ticks


def bubbles():
    for drag in ("false", "true"):
        bubble = f"minecraft:bubble_column[drag={drag}]"
        prefix = "bubble_down" if drag == "true" else "bubble_up"
        column = [[-2, -8, -2, 2, 8, 2, bubble]]
        yield scenario(prefix + "_interior", column, ticks(25), on_ground=False)
        yield scenario(prefix + "_keys", column,
                       ticks(6, jump=True) + ticks(6, sneak=True) + ticks(8, forward=1, sprint=True),
                       on_ground=False)
        yield scenario(prefix + "_fall_in", column, ticks(20),
                       start=(0.5, 10.0, 0.5), on_ground=False)
        # Different upper cells distinguish the versions' air-vs-empty-shape
        # tests. Each run also traverses the surface and re-enters the water.
        for label, upper in [("air", "minecraft:air"), ("water", "minecraft:water[level=0]"),
                             ("torch", "minecraft:torch"), ("roof", "minecraft:stone")]:
            blocks = [[-2, -5, -2, 2, 0, 2, bubble], [-2, 1, -2, 2, 1, 2, upper]]
            yield scenario(prefix + "_surface_" + label, blocks, ticks(18),
                           start=(0.5, 0.7, 0.5), on_ground=False)
        yield scenario(prefix + "_side_enter", [[0, -5, 0, 0, 4, 5, bubble]],
                       ticks(22, forward=1), start=(0.5, 0.5, -0.4), on_ground=False)
        yield scenario(prefix + "_side_exit", column, ticks(20, forward=1, sprint=True),
                       start=(0.5, 0.0, 1.8), on_ground=False)
        yield scenario(prefix + "_wide_overlap", [[0, -8, 0, 1, 8, 1, bubble]], ticks(20),
                       start=(1.0, 0.0, 1.0), on_ground=False)
    yield scenario("bubble_mixed_columns", [[0, -8, 0, 0, 8, 0, "minecraft:bubble_column[drag=false]"],
                                            [1, -8, 0, 1, 8, 0, "minecraft:bubble_column[drag=true]"]],
                   ticks(25), start=(1.0, 0.0, 0.5), on_ground=False)


def wet_boats():
    for level in (0, 1, 3, 7, 8):
        water = f"minecraft:water[level={level}]"
        blocks = [[-12, -8, -12, 12, 3, 12, water]]
        for label, controls in [("idle", ticks(30)),
                                ("steer", ticks(10, forward=1, strafe=1) + ticks(10, forward=-1) + ticks(10))]:
            yield dict(name=f"submerged_{level}_{label}", boat=True, start=[0.5, 0.0, 0.5],
                       blocks=blocks, ticks=controls)
    # Actual horizontal current, including the non-player normalization of
    # averaged currents and falling water's downward flow beside a solid face.
    for level in (1, 4, 7, 8):
        blocks = [[-12, -3, -12, 12, -1, 12, "minecraft:water[level=0]"],
                  [-12, 0, -12, 12, 0, 0, "minecraft:water[level=0]"],
                  [-12, 0, 1, 12, 0, 12, f"minecraft:water[level={level}]"],
                  [-12, -3, -1, 12, 2, -1, "minecraft:stone"]]
        yield dict(name=f"flowing_edge_{level}", boat=True, start=[0.5, 0.0, 0.5],
                   blocks=blocks, ticks=ticks(30))
    yield dict(name="fall_into_flowing", boat=True, start=[0.5, 2.0, 0.5],
               blocks=[[-12, -3, -12, 12, 0, 12, "minecraft:water[level=3]"]],
               ticks=ticks(20) + ticks(20, forward=1))
    # Modern floatBoat checks collision before snapping up to a surface.
    yield dict(name="water_entry_under_roof", boat=True, start=[0.5, 0.65, 0.5],
               blocks=[[-12, -3, -12, 12, 0, 12, "minecraft:water[level=0]"],
                       [-12, 1, -12, 12, 1, 12, "minecraft:stone"]], ticks=ticks(20))


if __name__ == "__main__":
    cases = list(ordinary_boats()) + list(wet_boats()) + list(bubbles())
    for case in cases:
        if not case.get("boat"):
            case["compare_fall_distance"] = True
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=1) + "\n")
    print(f"{sum(bool(c.get('boat')) for c in cases)} boat and {sum(not c.get('boat') for c in cases)} bubble scenarios")
