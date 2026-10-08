#!/usr/bin/env python3
"""Ordinary boat steering, buoyancy, inertia and block collision fixtures."""
import json
import sys
from pathlib import Path
from scenarios import ticks


def scenarios():
    water = [[-12, -2, -12, 12, -1, 12, "minecraft:water[level=0]"]]
    for forward in (-1, 0, 1):
        for strafe in (-1, 0, 1):
            yield dict(name=f"water_{forward}_{strafe}", boat=True, start=[0.5, -0.05, 0.5], blocks=water,
                       ticks=ticks(20, forward=forward, strafe=strafe) + ticks(25))
    for yaw in (35.57, -123.0, 180.0):
        yield dict(name=f"water_yaw_{yaw}", boat=True, start=[0.5, -0.05, 0.5], yaw=yaw, blocks=water,
                   ticks=ticks(15, forward=1) + ticks(10, forward=1, strafe=1) + ticks(25))
    yield dict(name="water_wall", boat=True, start=[0.5, -0.05, 0.5],
               blocks=water + [[-4, -2, 3, 4, 2, 3, "minecraft:stone"]], ticks=ticks(35, forward=1)+ticks(10))
    yield dict(name="fall_into_water", boat=True, start=[0.5, 1.5, 0.5], blocks=water,
               ticks=ticks(20)+ticks(20, forward=1)+ticks(10))
    for block in ("stone", "ice", "blue_ice"):
        yield dict(name="land_"+block, boat=True, start=[0.5, 0.0, 0.5],
                   blocks=[[-12,-1,-12,12,-1,12,"minecraft:"+block]],
                   ticks=ticks(15, forward=1)+ticks(10, strafe=-1)+ticks(15))


if __name__ == "__main__":
    cases = list(scenarios())
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=1)+"\n")
    print(f"{len(cases)} boat scenarios")
