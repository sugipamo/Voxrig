#!/usr/bin/env python3
"""Export original QUICK_MOVE routing/slot/codec facts on pinned official JARs.
Runs primitive contexts sequentially, 512 MiB heap and one processor. Does not
start a world/server or replace Minecraft methods. Official JARs stay local.
"""
import argparse,gzip,json,os,subprocess
from pathlib import Path
from export_regular_clicks import ROOT,VERSIONS,digest,encoded,original_modern_classpath

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads',type=Path,required=True)
    parser.add_argument('--modern-classpath-file',type=Path,required=True)
    parser.add_argument('--runtime-output',type=Path,required=True)
    parser.add_argument('--normalize-only',action='store_true')
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args();args.runtime_output.mkdir(parents=True,exist_ok=True)
    cp=os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    outputs={};runs=[]
    for version,(jar_sha1,mapping_sha256) in VERSIONS.items():
        jar=args.downloads/f'{version}-server.jar';mapping=args.downloads/f'{version}-server-mappings.txt'
        if digest(jar.read_bytes(),'sha1')!=jar_sha1 or digest(mapping.read_bytes())!=mapping_sha256:raise SystemExit('original input hash differs: '+version)
        hashes=original_modern_classpath(jar,cp) if version=='1.21.11' else {digest(jar.read_bytes()):jar.name}
        raw_path=args.runtime_output/f'{version}-raw.json'
        if not args.normalize_only:
            with (args.runtime_output/f'{version}-export.log').open('w') as log:
                result=subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(jar.resolve()) if version=='1.16.1' else cp,str(ROOT/'scripts/ExportInventoryTransfers.java'),version,str(raw_path.resolve())],cwd=args.runtime_output.resolve(),stdout=log,stderr=subprocess.STDOUT)
            if result.returncode:raise SystemExit(f'{version} original native exporter failed: {args.runtime_output}/{version}-export.log')
        raw=json.loads(raw_path.read_text())
        if raw['version']!=version or len(raw['routes'])!=(974 if version=='1.16.1' else 1504) or len(raw['equipment_slots'])!=5 or len(raw['packets'])!=18:raise SystemExit('native transfer coverage differs: '+version)
        profile=f'data/client_api/inventory_transfer_profiles-{version}.json';cases=f'data/client_api/inventory_transfer_cases-{version}.json.gz';packets=f'data/client_api/inventory_transfer_packets-{version}.json'
        outputs[profile]=encoded({'schema':1,'version':version,'routes':raw['routes'],'equipment_slots':raw['equipment_slots']})
        outputs[cases]=gzip.compress(encoded({'version':version,'cases':raw['cases']},compact=True),mtime=0)
        outputs[packets]=encoded(raw['packets'])
        runs.append({'version':version,'original_server_jar_sha1':jar_sha1,'mappings_sha256':mapping_sha256,'original_classpath_entries_sha256':hashes,'raw_output_sha256':digest(raw_path.read_bytes()),'default_item_routes':len(raw['routes']),'native_cases':len(raw['cases']),'codec_roundtrips':len(raw['packets']),'files_sha256':{p:digest(outputs[p]) for p in [profile,cases,packets]}})
    outputs['data/client_api/inventory_transfer_source.json']=encoded({'schema':1,'authority':'Actual unchanged menu clicked QUICK_MOVE, actual per-slot before/after/return, default item auto-equipment routes and original packet/hash codecs on pinned official server JARs. No native method bodies redistributed.','context':'Unspawned skeletal native ServerPlayer, actual Inventory/EntityEquipment and default flags. Actual native ServerPlayerGameMode is constructed for both versions; original Entity firstTick=true and player EntityType preserve unspawned context. Context providers do not replace native item/menu/equipment algorithms. No network/mode/session/ownership/recovery proof.','scope':'All nonempty registered default item routes from main and hotbar to empty player menu, ordinary storage nine constructors, merge/blocked/equipment-occupied fixtures, source armor/offhand and multi-step auto-equipment transfer; default item data only, including exact original legacy constructor NBT (such as Damage=0) rather than assuming all default items have no NBT. Requested source is distinct from actual native setter result.','generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in ['scripts/ExportInventoryTransfers.java','scripts/export_inventory_transfers.py','scripts/export_regular_clicks.py']},'runs':runs})
    for path,data in outputs.items():
        target=ROOT/path
        if args.check:
            if target.read_bytes()!=data:raise SystemExit('generated native transfer evidence differs: '+path)
        else:target.write_bytes(data)
    print('native transfer evidence verified' if args.check else 'native transfer evidence generated')
if __name__=='__main__':main()
