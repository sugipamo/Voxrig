#!/usr/bin/env python3
"""Boat client movement and bubble interiors; server launch uses real packets."""
import json
from pathlib import Path
import sys
from scenarios import ticks


def scenarios():
    for down in [False, True]:
        column = [[-8, -8, -8, 8, -1, 8, 'minecraft:bubble_column[drag=' + str(down).lower() + ']']]
        for label, start in [('surface', -.05), ('submerged', -2.3)]:
            for forward in [0, 1]:
                yield dict(name=f'{label}_{down}_{forward}', boat=True, boat_bubbles=True,
                           start=[.5, start, .5], blocks=column,
                           ticks=ticks(30, forward=forward) + ticks(5))
    # Instrumented input to original native movement, without claiming this
    # primitive fixture establishes packet receipt or the server bubble timer.
    for velocity in [2.7, -.7]:
        controls = ticks(40)
        controls[8]['received_boat_velocity'] = [0, velocity, 0]
        yield dict(name=f'received_vertical_{velocity}', boat=True, boat_bubbles=True,
                   start=[.5, -.05, .5], blocks=[[-8, -8, -8, 8, -1, 8, 'minecraft:bubble_column[drag=false]']], ticks=controls)
    for axis in (0, 2):
        up = [-8, -8, -8, 8, -1, 8]
        down = up.copy()
        up[axis + 3] = 0
        down[axis] = 1
        controls = ticks(30, forward=1, strafe=1)
        controls[3]['received_boat_velocity'] = [.6, -.4, .5]
        yield dict(name=f'mixed_drag_axis_{axis}', boat=True, boat_bubbles=True,
                   start=[.5, -2.3, .5], blocks=[up + ['minecraft:bubble_column[drag=false]'],
                                               down + ['minecraft:bubble_column[drag=true]']], ticks=controls)
    for cap in ('minecraft:water[level=0]', 'minecraft:stone_slab[type=bottom,waterlogged=false]',
                'minecraft:oak_slab[type=bottom,waterlogged=true]', 'minecraft:air'):
        yield dict(name='capped_' + cap, boat=True, boat_bubbles=True,
                   start=[.5, -1.4, .5], blocks=[[-8, -8, -8, 8, -1, 8, 'minecraft:bubble_column[drag=false]'],
                                               [-8, 0, -8, 8, 0, 8, cap]], ticks=ticks(30))


if __name__ == '__main__':
    cases = list(scenarios())
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=2) + '\n')
    print(len(cases), 'boat bubble scenarios')
