#!/usr/bin/env python3
"""Audit original enum ordinals and controlling-passenger declarations, not gameplay."""
import argparse
import hashlib
import io
import os
import json
import re
import subprocess
import zipfile
from pathlib import Path
from run_entity_pose_oracle import mappings
from movement_oracle.run import VERSIONS, classpath


def digest(data, algorithm='sha256'):
    return hashlib.new(algorithm, data).hexdigest()


class Reader:
    def __init__(self, data):
        self.data, self.offset = data, 0

    def take(self, size):
        result = self.data[self.offset:self.offset+size]
        if len(result) != size:
            raise ValueError('truncated original class')
        self.offset += size
        return result

    def u2(self):
        return int.from_bytes(self.take(2), 'big')

    def u4(self):
        return int.from_bytes(self.take(4), 'big')


def declarations(data):
    reader = Reader(data)
    if reader.u4() != 0xcafebabe:
        raise ValueError('class magic')
    reader.take(4)
    pool = [None] * reader.u2()
    index = 1
    while index < len(pool):
        kind = reader.take(1)[0]
        if kind == 1:
            pool[index] = reader.take(reader.u2()).decode('utf-8', errors='replace')
        elif kind in (7, 8, 16, 19, 20):
            pool[index] = reader.u2()
        elif kind in (3, 4, 9, 10, 11, 12, 17, 18):
            reader.take(4)
        elif kind in (5, 6):
            reader.take(8)
            index += 1
        elif kind == 15:
            reader.take(3)
        else:
            raise ValueError(('constant pool kind', kind))
        index += 1
    reader.u2()
    own, parent = reader.u2(), reader.u2()
    reader.take(2 * reader.u2())

    def members():
        result = []
        for _ in range(reader.u2()):
            access, name, descriptor = reader.u2(), reader.u2(), reader.u2()
            attributes = {}
            for _ in range(reader.u2()):
                key = pool[reader.u2()]
                attributes[key] = digest(reader.take(reader.u4()))
            result.append(dict(name=pool[name], descriptor=pool[descriptor], access=access,
                               attribute_hashes=attributes))
        return result

    fields, methods = members(), members()
    return dict(name=pool[pool[own]], parent=None if not parent else pool[pool[parent]],
                fields=fields, methods=methods, class_sha256=digest(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    source = root / 'CapabilityEnums.java'
    classes = args.work.resolve() / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    subprocess.run(['javac', '-d', str(classes), str(source)], check=True)
    result = dict(schema=1, authority='Read-only original enum runtime and class declarations. Controller and posture classification is a method/branch audit, not a numerical oracle, real connection, complete mount physics or server-current observation.',
                  runner_sha256=digest(Path(__file__).read_bytes()), enum_harness_sha256=digest(source.read_bytes()), versions={})
    for version, (jar_sha, mapping_sha, _) in VERSIONS.items():
        jar = args.downloads / (version + '-server.jar')
        mapping = args.downloads / (version + '-server-mappings.txt')
        assert digest(jar.read_bytes(), 'sha1') == jar_sha
        assert digest(mapping.read_bytes(), 'sha1') == mapping_sha
        table = mappings(mapping)
        reverse = {item['class'].replace('.', '/'): name for name, item in table.items()}
        with zipfile.ZipFile(jar) as outer:
            bundled = [name for name in outer.namelist()
                       if name.startswith('META-INF/versions/') and name.endswith('.jar')]
            game_data = outer.read(bundled[0]) if bundled else jar.read_bytes()
        controllers = []
        hooks = []
        selected_hooks = {"getRiddenInput", "getRiddenSpeed", "travelRidden", "canSimulateMovement", "isControlledByLocalInstance", "onPlayerJump", "handleStartJump", "handleStopJump", "canAddPassenger", "canFitPassenger", "startSleeping", "stopSleeping", "updatePlayerPose", "getDesiredPose", "isAutoSpinAttack"}
        primitives = dict(void="V", boolean="Z", byte="B", char="C", short="S", int="I", long="J", float="F", double="D")
        def descriptor_type(name):
            if name.endswith("[]"):
                return "[" + descriptor_type(name[:-2])
            return primitives.get(name) or "L" + (table[name]["class"] if name in table else name).replace(".", "/") + ";"
        native_classes = {}
        with zipfile.ZipFile(io.BytesIO(game_data)) as game:
            for name, item in table.items():
                if not name.startswith('net.minecraft.world.entity.'):
                    continue
                path = item['class'].replace('.', '/') + '.class'
                if path not in game.namelist():
                    continue
                native = declarations(game.read(path))
                native_classes[name] = native
                for line in item['members']:
                    matched = re.fullmatch(r'(?:\d+:\d+:)?(\S+) (\w+)\((.*?)\)(?::\d+:\d+)? -> (\S+)', line)
                    if not matched or matched[2] not in selected_hooks:
                        continue
                    arguments = matched[3].split(',') if matched[3] else []
                    descriptor = '(' + ''.join(map(descriptor_type, arguments)) + ')' + descriptor_type(matched[1])
                    methods = [method for method in native['methods'] if method['name'] == matched[4] and method['descriptor'] == descriptor]
                    if methods:
                        hooks.append(dict(class_name=name, hook=matched[2], original_method=methods[0], class_sha256=native['class_sha256']))
                names = [line.split(' -> ')[1] for line in item['members']
                         if ' getControllingPassenger() -> ' in line]
                matching = [method for method in native['methods']
                            if method['name'] in names and method['descriptor'].startswith('()')]
                if matching:
                    controllers.append(dict(class_name=name,
                        parent=reverse.get(native['parent'], native['parent']),
                        class_sha256=native['class_sha256'], declarations=matching))
        game, libraries = classpath(version, jar, args.work / version)
        assert digest(game.read_bytes()) == digest(game_data)
        classpath_hashes = {str(path.relative_to(args.work)) if path.is_relative_to(args.work)
                            else path.name: digest(path.read_bytes()) for path in libraries}
        owners = ['net.minecraft.world.entity.Pose',
                  'net.minecraft.network.protocol.game.ServerboundPlayerCommandPacket$Action']
        original_names = [table[owner]['class'] for owner in owners]
        command = ['java', '-cp', os.pathsep.join(map(str, [classes, *libraries])),
                   'voxrig.oracle.CapabilityEnums', *original_names]
        output = subprocess.check_output(command, text=True)
        enums = {owner: [] for owner in owners}
        for line in output.splitlines():
            original, ordinal, name = line.split('\t')
            enums[owners[original_names.index(original)]].append(dict(ordinal=int(ordinal), name=name))
        descendants = {}
        for name in ['net.minecraft.world.entity.animal.camel.CamelHusk',
                     'net.minecraft.world.entity.animal.nautilus.Nautilus',
                     'net.minecraft.world.entity.animal.nautilus.ZombieNautilus']:
            if name in native_classes:
                native = native_classes[name]
                descendants[name] = dict(parent=reverse.get(native['parent'], native['parent']),
                                         class_sha256=native['class_sha256'])
        result['versions'][version] = dict(original_server_jar_sha1=jar_sha,
            mappings_sha1=mapping_sha, original_inner_game_sha256=digest(game_data),
            enum_classpath_sha256=classpath_hashes,
            inspected_entity_class_count=len(native_classes), controlling_passenger_declarations=controllers, hook_declarations=hooks,
            enums=enums, enum_output_sha256=digest(output.encode()), additional_inheritance=descendants)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')


if __name__ == '__main__':
    main()
