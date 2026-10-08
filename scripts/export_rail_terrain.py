#!/usr/bin/env python3
"""Original registered dry rail shapes; sequential 512 MiB/CPU-one JVMs."""
import argparse
import gzip
import json
import os
from pathlib import Path
import subprocess
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['downloads', 'modern-classpath-file', 'runtime-output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve(); base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources = ['scripts/ExportStorageOutlines.java', 'scripts/ExportRailTerrain.java']
    named = base/'named-sources'; named.mkdir(exist_ok=True)
    for path in sources:
        (named/Path(path).name).write_text('package voxrig.oracle;\n'+(ROOT/path).read_text())
    compiler = base/'CompileOwnTool.java'; compiler.write_text(COMPILER)
    classes = base/'own-classes'
    if not args.normalize_only:
        with (base/'compile.log').open('w') as log:
            subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(compiler),cp,str(classes),
                            *(str(named/Path(p).name) for p in sources)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    outputs = {}; runs = []
    for version, (jar_sha, mapping_sha) in VERSIONS.items():
        jar = (args.downloads/(version+'-server.jar')).resolve()
        mapping = args.downloads/(version+'-server-mappings.txt')
        assert digest(jar.read_bytes(),'sha1') == jar_sha and digest(mapping.read_bytes()) == mapping_sha
        runtime = str(jar) if version=='1.16.1' else cp
        original = {digest(jar.read_bytes()):jar.name} if version=='1.16.1' else original_modern_classpath(jar,cp)
        raw = base/(version+'-raw.json')
        if not args.normalize_only:
            with (base/(version+'-run.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+runtime,
                                'voxrig.oracle.ExportRailTerrain',version,str(raw)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data = json.loads(raw.read_text()); assert data['version'] == version and data['states'] and data['collisions']
        assert all(s['state']['properties'].get('waterlogged','false')=='false' for s in data['states'])
        assert len({s['native_id'] for s in data['states']}) == len(data['states'])
        name = 'data/client_api/rail_terrain-'+version+'.json.gz'
        outputs[name] = gzip.compress(encoded(data, compact=True),mtime=0)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha,'mappings_sha256':mapping_sha,
                     'original_classpath_entries_sha256':original,'raw_output_sha256':digest(raw.read_bytes()),
                     'counts':{k:len(data[k]) for k in ['blocks','states','collisions','rays']},
                     'unique_collision_shapes':data['unique_collision_shapes'],'files_sha256':{name:digest(outputs[name])}})
    outputs['data/client_api/rail_terrain_source.json'] = encoded({
        'schema':1,
        'authority':'Original registered rail/powered_rail/detector_rail/activator_rail blocks, every dry state, native collision/outline/interaction shape getters and default friction/speed/jump factors. Original combined VoxelShape collision, Shapes.joinIsNotEmpty initial-body intersection and original BlockGetter.clip, without copied or replaced game bodies.',
        'scope':'Known registered dry rails only. No waterlogged, unknown properties/block, moving block, fluid, tool/effect/posture or vehicle semantics. Primitive collisions/clips are separate from full tick and live workflow verification.',
        'generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in sources+['scripts/export_rail_terrain.py', 'scripts/export_regular_clicks.py', 'scripts/export_item_properties.py']},
        'generated_compiler_sha256':digest(COMPILER.encode()),'runs':runs})
    for name, value in outputs.items():
        if args.check: assert (ROOT/name).read_bytes()==value,name
        else: (ROOT/name).write_bytes(value)
    print('Original dry rail terrain verified' if args.check else 'Original dry rail terrain generated',[(r['version'],r['counts']) for r in runs])


if __name__ == '__main__':
    main()
