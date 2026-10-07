#!/usr/bin/env python3
"""Generate scenarios.json for the movement oracle (deterministic)."""
import json
from pathlib import Path

FLOOR = [-6, -1, -6, 6, -1, 6]


def floor(state="minecraft:stone"):
    return [FLOOR + [state]]


def ticks(n, **control):
    control.setdefault("yaw", 0.0)
    return [dict(control) for _ in range(n)]


def scenario(name, blocks, controls, start=(0.5, 0.0, 0.5), **extra):
    value = {"name": name, "blocks": blocks, "start": list(start), "ticks": controls}
    value.update(extra)
    return value


def baseline():
    """Dry terrain the existing shared model already supports."""
    yield scenario("walk", floor(), ticks(20, forward=1) + ticks(15))
    yield scenario("walk_diagonal_yaw", floor(), ticks(20, forward=1, strafe=-1, yaw=35.57) + ticks(10, yaw=35.57))
    yield scenario("back_strafe", floor(), ticks(12, forward=-1, strafe=1, yaw=-123.0) + ticks(8, yaw=-123.0))
    yield scenario("jump_in_place", floor(), ticks(1, jump=True) + ticks(14))
    yield scenario("walk_hold_jump", floor(), ticks(40, forward=1, jump=True) + ticks(10))
    yield scenario("walk_into_wall", floor() + [[-2, 0, 3, 2, 1, 3, "minecraft:stone"]], ticks(25, forward=1) + ticks(5))
    yield scenario("step_onto_slab", floor() + [[-2, 0, 2, 2, 0, 4, "minecraft:stone_slab[type=bottom,waterlogged=false]"]],
                   ticks(25, forward=1) + ticks(10))
    yield scenario("walk_off_edge", [[-1, -1, -1, 1, -1, 1, "minecraft:stone"], [-4, -4, -4, 4, -4, 6, "minecraft:stone"]],
                   ticks(30, forward=1) + ticks(10))


def materials():
    for name in ["ice", "packed_ice", "blue_ice", "slime_block", "honey_block", "soul_sand"]:
        state = "minecraft:" + name
        yield scenario(f"{name}_walk", floor(state), ticks(30, forward=1) + ticks(30))
        yield scenario(f"{name}_jump", floor(state), ticks(25, forward=1, jump=True) + ticks(20))
    yield scenario("ice_turn", floor("minecraft:ice"), ticks(20, forward=1) + ticks(20, strafe=1, yaw=90.0))
    yield scenario("stone_to_ice", floor() + [[-6, -1, 2, 6, -1, 6, "minecraft:ice"]], ticks(20, forward=1) + ticks(25))
    yield scenario("slime_drop", floor("minecraft:slime_block"), ticks(40), start=(0.5, 3.0, 0.5), on_ground=False)
    yield scenario("slime_drop_sneak", floor("minecraft:slime_block"), ticks(40, sneak=True), start=(0.5, 3.0, 0.5),
                   on_ground=False)
    yield scenario("cobweb_walk", floor() + [[-1, 0, 2, 1, 1, 2, "minecraft:cobweb"]], ticks(40, forward=1) + ticks(10))
    yield scenario("cobweb_fall", floor() + [[0, 0, 0, 0, 3, 0, "minecraft:cobweb"]], ticks(40), start=(0.5, 4.0, 0.5),
                   on_ground=False)


def actions():
    yield scenario("sneak_walk", floor(), ticks(20, forward=1, sneak=True) + ticks(10))
    yield scenario("sneak_edge", [[-1, -1, -1, 1, -1, 1, "minecraft:stone"]], ticks(30, forward=1, sneak=True) + ticks(5))
    yield scenario("sneak_edge_diagonal", [[-1, -1, -1, 1, -1, 1, "minecraft:stone"]],
                   ticks(30, forward=1, strafe=1, sneak=True, yaw=20.0) + ticks(5, yaw=20.0))
    yield scenario("sneak_then_release_at_edge", [[-1, -1, -1, 1, -1, 1, "minecraft:stone"]],
                   ticks(25, forward=1, sneak=True) + ticks(10, forward=1))
    yield scenario("sprint", floor(), ticks(30, forward=1, sprint=True) + ticks(10))
    yield scenario("sprint_jump", floor(), ticks(40, forward=1, sprint=True, jump=True) + ticks(10))
    yield scenario("sprint_into_wall", floor() + [[-2, 0, 4, 2, 1, 4, "minecraft:stone"]],
                   ticks(30, forward=1, sprint=True) + ticks(5))
    yield scenario("sprint_strafe_only", floor(), ticks(20, strafe=1, sprint=True))
    yield scenario("sprint_sneak", floor(), ticks(20, forward=1, sprint=True, sneak=True))


def modifiers():
    yield scenario("speed_attribute", floor(), ticks(20, forward=1, jump=True),
                   attributes={"minecraft:generic.movement_speed": 0.2}, versions=["1.16.1"])
    yield scenario("speed_attribute", floor(), ticks(20, forward=1, jump=True),
                   attributes={"minecraft:movement_speed": 0.2}, versions=["1.21.11"])
    for effect, level in [("speed", 1), ("slowness", 2), ("jump_boost", 1)]:
        yield scenario(f"effect_{effect}", floor(), ticks(30, forward=1, jump=True, sprint=True),
                       effects={"minecraft:" + effect: level})
    modern = {
        "jump_strength": 0.6, "step_height": 1.0, "gravity": 0.04, "sneaking_speed": 0.8,
        "movement_efficiency": 1.0,
    }
    for name, value in modern.items():
        blocks = floor("minecraft:soul_sand" if name == "movement_efficiency" else "minecraft:stone")
        if name == "step_height":
            blocks += [[-2, 0, 2, 2, 0, 4, "minecraft:stone"]]
        yield scenario(f"attribute_{name}", blocks, ticks(25, forward=1, jump=name != "step_height",
                                                           sneak=name == "sneaking_speed") + ticks(10),
                       attributes={"minecraft:" + name: value}, versions=["1.21.11"])


def main():
    items = [*baseline(), *materials(), *actions(), *modifiers()]
    path = Path(__file__).with_name("scenarios.json")
    path.write_text(json.dumps(items, indent=1) + "\n")
    print(f"{len(items)} scenarios")


if __name__ == "__main__":
    main()
