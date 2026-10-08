#!/usr/bin/env python3
"""Generate climbing scenarios for the unchanged official movement oracle."""
import json
import sys
from pathlib import Path

from scenarios import floor, scenario, ticks


def climbing():
    for facing, yaw, wall in [
        ("north", 0.0, [0, 0, 1, 0, 8, 1]),
        ("south", 180.0, [0, 0, -1, 0, 8, -1]),
        ("east", 90.0, [-1, 0, 0, -1, 8, 0]),
        ("west", -90.0, [1, 0, 0, 1, 8, 0]),
    ]:
        ladder = f"minecraft:ladder[facing={facing},waterlogged=false]"
        vine = f"minecraft:vine[{facing}=true]"
        for name, state in [("ladder", ladder), ("vine", vine)]:
            blocks = floor() + [wall + ["minecraft:stone"], [0, 0, 0, 0, 7, 0, state]]
            yield scenario(f"{name}_{facing}_wall", blocks,
                           ticks(35, forward=1, yaw=yaw) + ticks(10, sneak=True, yaw=yaw))
    for name, state, versions in [
        ("ladder", "minecraft:ladder[facing=north,waterlogged=false]", None),
        ("vine", "minecraft:vine[north=true]", None),
        ("weeping_vines", "minecraft:weeping_vines[age=0]", None),
        ("weeping_vines_plant", "minecraft:weeping_vines_plant", None),
        ("twisting_vines", "minecraft:twisting_vines[age=0]", None),
        ("twisting_vines_plant", "minecraft:twisting_vines_plant", None),
        ("cave_vines", "minecraft:cave_vines[age=0,berries=false]", ["1.21.11"]),
        ("cave_vines_plant", "minecraft:cave_vines_plant[berries=false]", ["1.21.11"]),
    ]:
        blocks = floor() + [[0, 0, 0, 0, 7, 0, state]]
        extra = {} if versions is None else {"versions": versions}
        yield scenario(f"{name}_jump_sneak_release", blocks,
                       ticks(25, jump=True) + ticks(8, sneak=True) + ticks(8)
                       + ticks(8, jump=True, sneak=True) + ticks(10, forward=1), **extra)
        yield scenario(f"{name}_fall_sneak", blocks,
                       ticks(10) + ticks(12, sneak=True) + ticks(15),
                       start=(0.5, 5.0, 0.5), on_ground=False, **extra)
    ladder = "minecraft:ladder[facing=north,waterlogged=false]"
    for label, facing, opened in [("aligned", "north", "true"), ("mismatched", "south", "true"),
                                  ("closed", "north", "false")]:
        door = f"minecraft:oak_trapdoor[facing={facing},half=bottom,open={opened},powered=false,waterlogged=false]"
        yield scenario(f"trapdoor_{label}_jump", floor() + [[0, 0, 0, 0, 2, 0, ladder],
                       [0, 3, 0, 0, 3, 0, door]], ticks(50, jump=True) + ticks(10, sneak=True))
    yield scenario("ladder_waterlogged_wall", floor() + [[0, 0, 1, 0, 7, 1, "minecraft:stone"],
                   [0, 0, 0, 0, 6, 0, "minecraft:ladder[facing=north,waterlogged=true]"]],
                   ticks(40, forward=1) + ticks(10, forward=1, jump=True))
    for bottom, distance in [("false", 0), ("false", 1), ("true", 0), ("true", 1)]:
        state = f"minecraft:scaffolding[bottom={bottom},distance={distance},waterlogged=false]"
        blocks = floor() + [[0, 0, 0, 0, 5, 0, state]]
        label = f"scaffolding_{bottom}_{distance}"
        yield scenario(f"{label}_jump_descend", blocks,
                       ticks(65, jump=True) + ticks(8) + ticks(40, sneak=True) + ticks(10))
        yield scenario(f"{label}_land", blocks, ticks(35), start=(0.5, 7.0, 0.5), on_ground=False)
        yield scenario(f"{label}_side_enter", blocks, ticks(25, forward=1) + ticks(20, jump=True),
                       start=(0.5, 0.0, -1.5))
        yield scenario(f"{label}_walk_off_top", blocks, ticks(25, forward=1) + ticks(10),
                       start=(0.5, 6.0, 0.5))
    yield scenario("scaffolding_sneak_jump", floor() + [[0, 0, 0, 0, 5, 0,
                   "minecraft:scaffolding[bottom=false,distance=0,waterlogged=false]"]],
                   ticks(40, jump=True, sneak=True) + ticks(20, sneak=True))
    yield scenario("scaffolding_waterlogged", floor() + [[0, 0, 0, 0, 5, 0,
                   "minecraft:scaffolding[bottom=false,distance=0,waterlogged=true]"]],
                   ticks(50, jump=True) + ticks(20, sneak=True))


def main():
    cases = list(climbing())
    for case in cases:
        case["compare_fall_distance"] = True
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=1) + "\n")
    print(f"{len(cases)} climbing scenarios")


if __name__ == "__main__":
    main()
