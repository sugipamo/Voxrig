#!/usr/bin/env python3
"""Fingerprint the exclusion-only and respawn review against both official JARs.

Manual semantic conclusions are in docs/contact-scope-regression-audit.md. Hashes
bind that review; matching owners alone do not qualify a species numerically.
"""
import argparse
import json
import re
import subprocess
import zipfile
from pathlib import Path
from audit_contact_capabilities import CLIENT, ROOT, audit, digest
from audit_contact_envelopes import methods

SERVER = {
    '1.16.1': ('a412fd69db1f81db3f511c1463fd304675244077', '11120c39da4df293c4bd020896391fb9ddd6c2ba'),
    '1.21.11': ('64bb6d763bed0a9f1d632ec347938594144943ed', '5621e9253f05fd57872bbe7f8ddf5f9a7d525955'),
}
EXCLUDED = ['fox','skeleton','stray','wither_skeleton','enderman','wolf','horse','donkey','mule',
            'skeleton_horse','zombie_horse','cod','salmon','tropical_fish','pufferfish','husk','drowned','chest_minecart']
IRRELEVANT = ['item','experience_orb','arrow','spectral_arrow','trident']
SOURCES = ['src/client/entity.rs','src/client/entity/scope.rs','src/client/entity/owned_contacts.rs',
           'src/versions/java_1_16_1/client/play_player.rs','src/versions/java_1_21_11/client.rs',
           'src/versions/java_1_21_11/client/entity.rs','docs/contact-scope-regression-audit.md',
           'docs/common-respawn.md','scripts/audit_contact_scope_regression.py']


def ancestors(jar, roots):
    pending=set(roots); found=set()
    while pending:
        cls=pending.pop()
        if cls in found or cls.startswith('java.'):
            continue
        found.add(cls)
        raw=subprocess.check_output(['javap','-p','-classpath',str(jar),cls],text=True)
        header=next(line for line in raw.splitlines() if re.search(r'\b(class|interface)\b',line))
        parent=re.search(r' extends ([\w.$]+)',header)
        if parent:
            pending.add(parent[1])
    return found


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path)
    p.add_argument('--client-named',type=Path)
    p.add_argument('--servers',type=Path)
    p.add_argument('--work',type=Path)
    p.add_argument('--output',type=Path,default=ROOT/'docs/evidence/contact-scope-regression-20261025.json')
    p.add_argument('--check',action='store_true')
    a=p.parse_args()
    if a.check:
        e=json.loads(a.output.read_text())
        assert e['exclusion_only']==EXCLUDED and e['no_contact']==IRRELEVANT
        for source,value in e['source_sha256'].items():
            assert digest(ROOT/source)==value,source
        for v, hashes in SERVER.items():
            assert e['versions'][v]['server_jar_sha1']==hashes[0]
            assert e['versions'][v]['server_mappings_sha1']==hashes[1]
            assert e['versions'][v]['client_jar_sha1']==CLIENT[v][0]
        print('PASS: regression review domains, official inputs and source fingerprints')
        return
    if any(x is None for x in [a.downloads,a.client_named,a.servers,a.work]):
        p.error('--downloads, --client-named, --servers and --work are required')
    e=dict(schema=1,contract='decision A; receipt origin and explicit model ticks, no real-time server claim',
           exclusion_only=EXCLUDED,no_contact=IRRELEVANT,versions={})
    tool=a.downloads/'SpecialSource-1.11.4-shaded.jar'
    assert digest(tool)=='e2cab24b1c12400ad73b15972bb21e4273a0dc7081c8b3c136ddfdd824c78518'
    for v, (server_sha, mappings_sha) in SERVER.items():
        for suffix,expected in [('client.jar',CLIENT[v][0]),('client-mappings.txt',CLIENT[v][1])]:
            assert digest(a.downloads/f'{v}-{suffix}','sha1')==expected
        server=a.servers/f'{v}-server.jar';mapping=a.servers/f'{v}-server-mappings.txt'
        assert digest(server,'sha1')==server_sha
        assert digest(mapping,'sha1')==mappings_sha
        folder=a.work/v;folder.mkdir(parents=True,exist_ok=True)
        if v!='1.16.1':
            original=folder/'server-original.jar'
            with zipfile.ZipFile(server) as z:
                original.write_bytes(z.read(f'META-INF/versions/{v}/server-{v}.jar'))
        else:
            original=server
        named=folder/'server-named.jar'
        subprocess.check_call(['java','-jar',str(tool),'--in-jar',str(original),'--out-jar',str(named),'--srg-in',str(mapping)],stdout=subprocess.DEVNULL)
        client=a.client_named/v/'named.jar';rows=audit(client)
        roots=ancestors(client,{rows[n]['class_name'] for n in EXCLUDED+IRRELEVANT})
        roots.update(['net.minecraft.world.entity.EntityType','net.minecraft.world.entity.EntityDimensions',
                      'net.minecraft.client.multiplayer.ClientPacketListener','net.minecraft.client.multiplayer.MultiPlayerGameMode'])
        if v!='1.16.1':
            roots.update(['net.minecraft.world.entity.vehicle.minecart.OldMinecartBehavior',
                          'net.minecraft.world.entity.vehicle.minecart.NewMinecartBehavior',
                          'net.minecraft.world.entity.InterpolationHandler',
                          'net.minecraft.world.entity.animal.fish.Salmon$Variant'])
        server_roots={'net.minecraft.server.players.PlayerList','net.minecraft.server.level.ServerPlayer',
                      'net.minecraft.server.network.ServerGamePacketListenerImpl','net.minecraft.world.entity.player.Inventory',
                      'net.minecraft.world.entity.player.Player'}
        e['versions'][v]=dict(server_jar_sha1=server_sha,server_mappings_sha1=mappings_sha,
            client_jar_sha1=CLIENT[v][0],client_mappings_sha1=CLIENT[v][1],
            client_named_sha256=digest(client),server_named_sha256=digest(named),
            types={n:rows[n] for n in EXCLUDED+IRRELEVANT},
            client_method_sha256=methods(client,roots),server_method_sha256=methods(named,server_roots))
    e['source_sha256']={source:digest(ROOT/source) for source in SOURCES}
    a.output.write_text(json.dumps(e,indent=2)+'\n')
    print('PASS: recorded both official server/client exclusion and respawn method fingerprints')


if __name__=='__main__':
    main()
