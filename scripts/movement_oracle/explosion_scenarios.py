#!/usr/bin/env python3
"""Ordered explosion addition and velocity replacement before unchanged travel."""
import json
import sys
from pathlib import Path
from scenarios import scenario, ticks


def scenarios():
    floor = [[-12, -1, -12, 12, -1, 12, 'minecraft:stone']]
    a = dict(kind='explosion', value=[.125, .375, -.25])
    b = dict(kind='explosion', value=[-.0625, .125, .1875])
    v = dict(kind='velocity', value=[0., 0., 0.])
    for label, events in [('two', [a, b]), ('velocity_before', [v, a, b]),
                          ('velocity_between', [a, v, b]), ('velocity_after', [a, b, v]),
                          ('omitted_zero', [dict(kind='explosion', value=None),
                                            dict(kind='explosion', value=[0., 0., 0.])])]:
        for airborne in (False, True):
            controls = ticks(24, forward=1)
            controls[3]['motion_inputs'] = events
            controls[7]['motion_inputs'] = [b, a]
            yield scenario('explosion_' + label + ('_air' if airborne else '_ground'), floor, controls,
                           versions=['1.21.11'], start=(.5, 2. if airborne else 0., .5),
                           on_ground=not airborne, velocity=[.03125, -.125 if airborne else 0., 0.])


if __name__ == '__main__':
    Path(sys.argv[1]).write_text(json.dumps(list(scenarios()), indent=1) + '\n')
