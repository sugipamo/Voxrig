#!/usr/bin/env python3
"""Modern living honey callbacks: speed boundaries, visitation, scale and exit."""
import json
import struct
import sys
from pathlib import Path
from scenarios import scenario, ticks


def f32(value):
    return struct.unpack('>f', struct.pack('>f', value))[0]


def scenarios():
    floor = [[-8, -1, -8, 8, -1, 8, 'minecraft:stone']]
    wall = [[0, 0, -3, 0, 9, 3, 'minecraft:honey_block']]
    for speed in (-.079999, -.08, -.080001, -.129999, -.13, -.130001, -.6):
        yield scenario(f'honey_speed_{speed}', floor + wall, ticks(6),
                       start=(-.2375, 5., .5), velocity=[0., speed, .12])
    for scale in (.125, .25, .5, 1., 2., 4.):
        half = f32(f32(.6) * scale) / 2.
        for label, controls in [('fall', ticks(65)), ('sneak', ticks(65, sneak=True)),
                                ('leave', ticks(5) + ticks(30, strafe=-1))]:
            yield scenario(f'honey_scale_{scale}_{label}', floor + wall, controls,
                           start=(.0625-half, 3., .5), velocity=[0., -.3, .04],
                           attributes={'minecraft:scale': scale})
    for yaw in (0., 90., 180., -90.):
        yield scenario(f'honey_enter_{yaw}', floor + wall, ticks(40, forward=1, yaw=yaw),
                       start=(-.5, 4., .5), velocity=[.08, -.3, .04])
    # The honey wall clips one horizontal component. When that component
    # started faster, the final displacement has the opposite axis ordering.
    # Immediate honey/bubble callbacks must retain the attempted movement order.
    for honey_axis in (0, 2):
        bubble_axis = 2 - honey_axis
        for x_sign, z_sign in ((1, 1), (1, -1), (-1, 1), (-1, -1)):
            signs = [x_sign, 1, z_sign]
            honey_cell = [0, 0, 0]
            bubble_cell = [0, 0, 0]
            honey_cell[honey_axis] = signs[honey_axis]
            bubble_cell[bubble_axis] = signs[bubble_axis]
            blocks = floor + [
                [honey_cell[0], 0, honey_cell[2], honey_cell[0], 9, honey_cell[2],
                 'minecraft:honey_block'],
                [bubble_cell[0], 0, bubble_cell[2], bubble_cell[0], 9, bubble_cell[2],
                 'minecraft:bubble_column[drag=false]'],
            ]
            for faster in ('honey', 'bubble'):
                velocity = [signs[0] * .35, -.3, signs[2] * .35]
                axis = honey_axis if faster == 'honey' else bubble_axis
                velocity[axis] = signs[axis] * .45
                yield scenario(f'honey_axis_order_{honey_axis}_{x_sign}_{z_sign}_{faster}',
                               blocks, ticks(3), start=(.5, 4., .5), velocity=velocity)
    for label, blocks, start, velocity in [
        ('corner', wall, (-.2375, 4., -3.2375), [.04, -.3, .04]),
        ('multiple', [[0, 0, 0, 0, 9, 0, 'minecraft:honey_block'],
                      [0, 0, 1, 0, 9, 1, 'minecraft:honey_block']], (-.2375, 4., 1.), [0., -.3, .07]),
        ('inside_no_slide', wall, (.5, 4., .5), [0., -.3, 0.]),
        ('top_jump', wall, (.5, 9.9375, .5), [0., 0., 0.]),
        ('bubble_before_honey', wall + [[-1, 0, -3, -1, 9, 3, 'minecraft:bubble_column[drag=false]']], (-.2375, 4., .5), [0., -.3, .04]),
        ('honey_before_bubble', [[-1, 0, -3, -1, 9, 3, 'minecraft:honey_block'],
                                [0, 0, -3, 0, 9, 3, 'minecraft:bubble_column[drag=true]']], (.2375, 4., .5), [0., -.3, .04]),
    ]:
        yield scenario('honey_' + label, floor + blocks, ticks(25, jump=label=='top_jump'),
                       start=start, velocity=velocity, on_ground=label=='top_jump')
    for label, extra in [('gravity', {'attributes': {'minecraft:gravity': .02}}),
                         ('slow_falling', {'effects': {'minecraft:slow_falling': 0}})]:
        yield scenario('honey_' + label, floor + wall, ticks(25), start=(-.2375, 4., .5),
                       velocity=[0., -.3, .04], **extra)


if __name__ == '__main__':
    cases = list(scenarios())
    for case in cases:
        case.setdefault('on_ground', False)
        case.update(versions=['1.21.11'], compare_fall_distance=True, fall_distance=7.)
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=1) + '\n')
    print(f'{len(cases)} honey scenarios')
