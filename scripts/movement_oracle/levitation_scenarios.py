#!/usr/bin/env python3
"""Modern Ground levitation: levels, effect transitions, gravity and fluids."""
import json
import sys
from pathlib import Path
from scenarios import scenario, ticks

LEVITATION = 'minecraft:levitation'
SLOW = 'minecraft:slow_falling'


def scenarios():
    floor = [[-8, -1, -8, 8, -1, 8, 'minecraft:stone']]
    for amplifier in (0, 1, 4, 127, 254, 255):
        for label in ('floor', 'falling', 'ceiling', 'slow_falling'):
            blocks = floor + ([[-8, 3, -8, 8, 3, 8, 'minecraft:stone']] if label == 'ceiling' else [])
            yield scenario(f'levitation_{amplifier}_{label}', blocks, ticks(20, forward=1),
                           start=(.5, 0. if label == 'floor' else 1., .5),
                           velocity=[0., -.4 if label in ('falling', 'slow_falling') else 0., 0.],
                           on_ground=label == 'floor',
                           effects={LEVITATION: amplifier, **({SLOW: 0} if label == 'slow_falling' else {})})
    for label, updates in [
        ('add_remove', [(5, {LEVITATION: 0}), (20, {LEVITATION: None})]),
        ('change_levels', [(1, {LEVITATION: 0}), (8, {LEVITATION: 4}), (14, {LEVITATION: 1}), (22, {LEVITATION: None})]),
        ('slow_overlap', [(2, {SLOW: 0}), (6, {LEVITATION: 1}), (18, {LEVITATION: None}), (28, {SLOW: None})]),
        ('ceiling_release', [(0, {LEVITATION: 1}), (18, {LEVITATION: None}), (28, {LEVITATION: 0})]),
    ]:
        controls = ticks(40, strafe=1)
        for index, effects in updates:
            controls[index]['effect_updates'] = effects
        blocks = floor + ([[-8, 3, -8, 8, 3, 8, 'minecraft:stone']] if label == 'ceiling_release' else [])
        yield scenario('levitation_' + label, blocks, controls, start=(.5, 1., .5), on_ground=False)
    for gravity in (0., .005, .08, .3):
        yield scenario(f'levitation_gravity_{gravity}', floor, ticks(12) + ticks(20, effect_updates={LEVITATION: None}),
                       start=(.5, 1., .5), effects={LEVITATION: 1, SLOW: 0},
                       attributes={'minecraft:gravity': gravity}, on_ground=False, velocity=[0., -.4, 0.])
    for fluid in ('water', 'lava'):
        for slow in (False, True):
            yield scenario(f'levitation_{fluid}_{slow}', floor + [[-8, 0, -8, 8, 3, 8, f'minecraft:{fluid}[level=0]']],
                           ticks(25, forward=1, jump=True), start=(.5, 1., .5), on_ground=False,
                           effects={LEVITATION: 4, **({SLOW: 0} if slow else {})})


if __name__ == '__main__':
    cases = list(scenarios())
    for case in cases:
        case.update(versions=['1.21.11'], compare_fall_distance=True, fall_distance=7.)
    Path(sys.argv[1]).write_text(json.dumps(cases, indent=1) + '\n')
    print(f'{len(cases)} levitation scenarios')
