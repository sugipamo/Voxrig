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
    yield scenario("sprint_jump", [[-6, -1, -6, 6, -1, 14, "minecraft:stone"]],
                   ticks(40, forward=1, sprint=True, jump=True) + ticks(10), start=(0.5, 0.0, -5.5))
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


def terrain():
    """Natural terrain shapes and edge cases of the supporting block."""
    def walk(name, extra, controls=None, start=(0.5, 0.0, 0.5), **kw):
        yield scenario(name, floor() + extra, controls or ticks(25, forward=1, jump=True) + ticks(10), start=start, **kw)
    shapes = {
        "oak_fence": "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]",
        "fence_row": "minecraft:oak_fence[east=true,north=false,south=false,waterlogged=false,west=true]",
        "cobblestone_wall": "minecraft:cobblestone_wall",
        "snow_2": "minecraft:snow[layers=2]",
        "snow_7": "minecraft:snow[layers=7]",
        "white_carpet": "minecraft:white_carpet",
        "farmland": "minecraft:farmland[moisture=0]",
        "oak_leaves": "minecraft:oak_leaves[distance=7,persistent=true]",
        "top_slab": "minecraft:stone_slab[type=top,waterlogged=false]",
        "chest": "minecraft:chest",
        "oak_trapdoor_closed": "minecraft:oak_trapdoor[facing=north,half=bottom,open=false,powered=false,waterlogged=false]",
        "gravel": "minecraft:gravel",
        "cactus": "minecraft:cactus[age=0]",
        "lantern": "minecraft:lantern[hanging=false]",
        "sweet_berry_bush": "minecraft:sweet_berry_bush[age=3]",
        "red_bed": "minecraft:red_bed[facing=south,occupied=false,part=foot]",
        "glass_pane": "minecraft:glass_pane[east=true,north=false,south=false,waterlogged=false,west=true]",
        "oak_door": "minecraft:oak_door[facing=north,half=lower,hinge=left,open=false,powered=false]",
    }
    for name, state in shapes.items():
        # The server tramples farmland on landing (randomly, server-side only).
        controls = ticks(35, forward=1) if name == "farmland" else None
        for blocks, label in [([[-2, 0, 2, 2, 0, 3, state]], "row"), ([[0, 0, 2, 0, 0, 2, state]], "single")]:
            yield from walk(f"terrain_{name}_{label}", blocks, controls)
    open_trapdoor = "minecraft:oak_trapdoor[facing=south,half=bottom,open=true,powered=false,waterlogged=false]"
    yield from walk("trapdoor_open_no_ladder", [[0, 0, 2, 0, 0, 2, open_trapdoor]])
    for version, grass, path in [("1.16.1", "minecraft:grass", "minecraft:grass_path"),
                                 ("1.21.11", "minecraft:short_grass", "minecraft:dirt_path")]:
        yield from walk("terrain_grass_row", [[-2, 0, 2, 2, 0, 3, grass]], versions=[version])
        yield from walk("terrain_path_floor", [[-6, -1, -6, 6, -1, 6, path]], versions=[version])
    for facing in ["north", "south", "east", "west"]:
        stairs = f"minecraft:oak_stairs[facing={facing},half=bottom,shape=straight,waterlogged=false]"
        for yaw in [0.0, 90.0, 180.0, -90.0, 30.0]:
            yield from walk(f"stairs_{facing}_{int(yaw)}", [[-1, 0, 1, 1, 0, 1, stairs], [-1, 0, 2, 1, 1, 4, "minecraft:stone"]],
                            ticks(30, forward=1, yaw=yaw) + ticks(5, yaw=yaw))
    yield from walk("ceiling_slab_jump", [[-1, 2, -1, 1, 2, 1, "minecraft:stone_slab[type=bottom,waterlogged=false]"]],
                    ticks(20, jump=True, forward=1) + ticks(10))
    yield from walk("low_ceiling_jump", [[-2, 2, -2, 2, 2, 2, "minecraft:stone"]], ticks(20, jump=True) + ticks(5))
    yield scenario("edge_between_ice_and_stone", [[-6, -1, -6, 0, -1, 6, "minecraft:ice"], [1, -1, -6, 6, -1, 6, "minecraft:stone"]],
                   ticks(20, strafe=-1, forward=1, yaw=10.0) + ticks(30, yaw=10.0), start=(0.2, 0.0, -3.5))
    yield scenario("fence_top_walk", [[-3, 0, 0, 3, 0, 0, "minecraft:oak_fence[east=true,north=false,south=false,waterlogged=false,west=true]"]],
                   ticks(20, strafe=1, yaw=0.0) + ticks(10), start=(0.5, 1.5, 0.5))
    yield scenario("sneak_fence_top", [[-1, 0, 0, 1, 0, 0, "minecraft:oak_fence[east=true,north=false,south=false,waterlogged=false,west=true]"]],
                   ticks(30, forward=1, sneak=True) + ticks(5, sneak=True), start=(0.5, 1.5, 0.5))
    yield scenario("sneak_slab_edge", [[-1, 0, -1, 1, 0, 1, "minecraft:stone_slab[type=bottom,waterlogged=false]"]] + floor(),
                   ticks(30, forward=-1, sneak=True, yaw=45.0), start=(0.5, 0.5, 0.5))
    yield scenario("sneak_under_slab", floor() + [[-3, 1, 2, 3, 1, 5, "minecraft:stone_slab[type=top,waterlogged=false]"]],
                   ticks(10, forward=1, sneak=True) + ticks(15, forward=1) + ticks(10))
    yield scenario("push_out_of_wall", floor() + [[0, 0, 1, 0, 1, 1, "minecraft:stone"]], ticks(20), start=(0.5, 0.0, 0.9))
    yield scenario("push_out_corner", floor() + [[0, 0, 1, 1, 1, 1, "minecraft:stone"], [1, 0, 0, 1, 1, 0, "minecraft:stone"]],
                   ticks(20, forward=1, yaw=-45.0), start=(0.75, 0.0, 0.75))
    yield scenario("bed_drop", floor("minecraft:stone") + [[-1, 0, -1, 1, 0, 1, "minecraft:red_bed[facing=south,occupied=false,part=foot]"]],
                   ticks(30), start=(0.5, 3.0, 0.5), on_ground=False)
    yield scenario("berry_bush_walk", floor() + [[-1, 0, 2, 1, 0, 3, "minecraft:sweet_berry_bush[age=1]"]], ticks(40, forward=1))
    yield scenario("slime_walk_off", [[-6, -1, -6, 6, -1, 6, "minecraft:stone"], [-6, -1, -6, 6, -1, 0, "minecraft:slime_block"]],
                   ticks(30, forward=1) + ticks(10), start=(0.5, 0.0, -3.5))
    yield scenario("soul_sand_edge_fall", [[-1, -1, -1, 1, -1, 1, "minecraft:soul_sand"], [-6, -3, -6, 6, -3, 6, "minecraft:stone"]],
                   ticks(30, forward=1, sprint=True) + ticks(10))
    yield scenario("long_ice_sprint_jump", [[-6, -1, -12, 6, -1, 12, "minecraft:packed_ice"]],
                   ticks(30, forward=1, sprint=True, jump=True) + ticks(15), start=(0.5, 0.0, -10.5))


def modifiers_extra():
    yield scenario("speed_and_slowness_sprint", floor(), ticks(30, forward=1, sprint=True),
                   effects={"minecraft:speed": 2, "minecraft:slowness": 0})
    yield scenario("slow_falling_drop", floor(), ticks(40), start=(0.5, 4.0, 0.5), on_ground=False,
                   effects={"minecraft:slow_falling": 0})
    yield scenario("blindness_sprint", floor(), ticks(20, forward=1, sprint=True), effects={"minecraft:blindness": 0})
    yield scenario("weaving_cobweb", floor() + [[-1, 0, 2, 1, 1, 2, "minecraft:cobweb"]], ticks(30, forward=1),
                   effects={"minecraft:weaving": 0}, versions=["1.21.11"])
    yield scenario("strafe_sprint_jump_turning", floor(), [dict(forward=1, strafe=(1 if i % 10 < 5 else -1), sprint=True,
                   jump=i % 7 == 0, yaw=float(i * 9 - 180)) for i in range(40)])


def main():
    items = [*baseline(), *materials(), *actions(), *modifiers(), *terrain(), *modifiers_extra()]
    path = Path(__file__).with_name("scenarios.json")
    path.write_text(json.dumps(items, indent=1) + "\n")
    print(f"{len(items)} scenarios")


if __name__ == "__main__":
    main()
