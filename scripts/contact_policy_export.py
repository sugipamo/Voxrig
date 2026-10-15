#!/usr/bin/env python3
"""Development-only const-table JSON export and generated documentation check.

python3 -B scripts/contact_policy_export.py > /tmp/contact-policy.json
python3 -B scripts/contact_policy_export.py --write-docs
python3 -B scripts/contact_policy_export.py --check
No SDK API, runtime configuration, Rust generation, or oracle regeneration.
"""
import argparse
import difflib
import hashlib
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def table_export():
    output = subprocess.check_output([
        'cargo', 'test', '--locked', '--lib', '-j1',
        'client::entity::contact_policy::audit::export_contact_capability_policy',
        '--', '--exact', '--nocapture', '--test-threads=1',
    ], cwd=ROOT, text=True)
    exports = [line.split('CONTACT_POLICY=', 1)[1]
               for line in output.splitlines() if 'CONTACT_POLICY=' in line]
    if len(exports) != 1:
        raise ValueError('expected exactly one Rust const-table export')
    policy = json.loads(exports[0])
    if policy['schema_version'] != 1:
        raise ValueError('unsupported contact-policy export schema')
    payload = {key: value for key, value in policy.items() if key != 'policy_digest'}
    canonical = json.dumps(payload, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
    if hashlib.sha256(canonical.encode()).hexdigest() != policy['policy_digest']:
        raise ValueError('invalid contact-policy export digest')
    return policy


def classifications(policy):
    return {version: {name: row['classifications'] for name, row in data['types'].items()}
            for version, data in policy['versions'].items()}


def targets(policy, version, scenario):
    return {name for name, row in policy['versions'][version]['types'].items()
            if row['lanes']['OwnedWalking']['scenarios'] == scenario}


def packet_targets(policy, version):
    result = defaultdict(set)
    for name, row in policy['versions'][version]['types'].items():
        lane = row['lanes']['OwnedWalking']
        for rule in policy['packet_profiles'][lane['packet_profile']]['exact']:
            if rule['decision'] == 'Irrelevant(PacketAudit)':
                result[rule['packet']].add(name)
    return dict(result)


def coverage_markdown(policy):
    lines = ['| Version | Types | Walking | Incoming | Solid | Geometry | Metadata | Status | Animation |',
             '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |']
    fields = ['walking', 'incoming', 'solid', 'geometry', 'metadata', 'status', 'animation']
    for version, rows in classifications(policy).items():
        grouped = defaultdict(list)
        for name, effects in rows.items():
            grouped[tuple(effects[field] for field in fields)].append(name)
        for values, names in sorted(grouped.items()):
            lines.append('| ' + ' | '.join([version, ', '.join(names), *values]) + ' |')
    return '\n'.join(lines)


def packets_markdown(policy):
    lines = ['| Version | Packet key (not wire ID) | Qualified targets |', '| --- | --- | --- |']
    for version in policy['versions']:
        for packet, names in sorted(packet_targets(policy, version).items()):
            lines.append(f"| {version} | {packet} | {', '.join(sorted(names))} |")
    return '\n'.join(lines)


def envelopes_markdown(policy):
    lines = ['| Version | Profile | Eligible types | Lanes | Effects | Revocations |',
             '| --- | --- | --- | --- | --- | --- |']
    for version in policy['versions']:
        for name, locality in policy['locality_profiles'].items():
            if not locality['lanes']:
                continue
            names = [entity for entity, row in policy['versions'][version]['types'].items()
                     if any(row['lanes'][lane]['locality'] == name for lane in locality['lanes'])]
            revokes = locality['revokes']
            # Lossless grouping, checked against the fully expanded export.
            status = [f'Status({n})' for n in range(-128, 128)]
            animation = [f'Animation({n})' for n in range(256)]
            if set(status + animation) <= set(revokes):
                other = [p for p in revokes if p not in status + animation]
                summary = ', '.join(other + ['Status(-128..127)', 'Animation(0..255)'])
            else:
                summary = ', '.join(revokes)
            lines.append('| ' + ' | '.join([version, name, ', '.join(names),
                ', '.join(locality['lanes']), ', '.join(locality['effects']), summary]) + ' |')
    return '\n'.join(lines)


def selectors_markdown(policy):
    lines = ['| Version | Owned tick selector | Qualified species |', '| --- | --- | --- |']
    for version, data in policy['versions'].items():
        groups = defaultdict(list)
        for name, row in data['types'].items():
            lane = row['lanes']['OwnedWalking']
            if lane['modeled']:
                groups[lane['selectors']['tick']].append(name)
        for selector, names in sorted(groups.items()):
            lines.append(f"| {version} | {selector} | {', '.join(names)} |")
    return '\n'.join(lines)


def manifest_markdown(policy):
    lines = [f"Policy SHA-256: `{policy['policy_digest']}`.",
             f"Expanded present keys: {policy['expanded_present_keys']['count']:,}; "
             f"versioned entities: {policy['expanded_present_keys']['versioned_entities']}.", '',
             '| Version | Scenario ID | Assigned types |', '| --- | --- | --- |']
    for version in policy['versions']:
        for scenario in policy['scenarios']:
            names = targets(policy, version, scenario['id'])
            if names:
                lines.append(f"| {version} | {scenario['id']} | {', '.join(sorted(names))} |")
    lines += ['', '| Version | Public geometry selector | Types |', '| --- | --- | --- |']
    for version, data in policy['versions'].items():
        groups = defaultdict(list)
        for name, row in data['types'].items():
            selector = row['lanes']['PublicGeometry']['selectors']['geometry']
            if selector != 'NotApplicable':
                groups[selector].append(name)
        for selector, names in sorted(groups.items()):
            lines.append(f"| {version} | {selector} | {', '.join(names)} |")
    lines += ['', '| Evidence ID | Artifact |', '| --- | --- |']
    lines += [f"| {row['id']} | {row['artifact'] or 'none'} |" for row in policy['evidence']]
    return '\n'.join(lines)


def check_review_enumerations(policy):
    # This paragraph defines the scope of the handwritten side-effect review.
    # Keep its species set checked even after regenerating the coverage table.
    doc = (ROOT / 'docs/contact-capability-audit.md').read_text()
    match = re.search(r'Both versions qualify (.*?)\. All inherit', doc, re.S)
    if match is None:
        raise ValueError('missing ordinary-mob review scope enumeration')
    names = re.split(r',\s*|\s+and\s+', match[1].strip())
    observed = {re.sub(r'(?<=[a-z])(?=[A-Z])', '_', name).lower().strip() for name in names}
    historical = json.loads((ROOT / 'docs/evidence/contact-capabilities-20261014.json').read_text())
    for version, rows in classifications(policy).items():
        previous = {name for name, row in historical['versions'][version]['types'].items()
                    if row['classifications']['walking'] == 'Qualified'}
        current = {name for name, effects in rows.items() if effects['walking'] == 'Qualified'}
        if observed != current - previous:
            raise ValueError(f'{version}: handwritten ordinary-mob review species differ from table')
    rejected = set(re.findall(r'^\| `([a-z_]+)` \|', doc, re.M))
    evidence = json.loads((ROOT / 'docs/evidence/contact-capabilities-20261015.json').read_text())
    for version, data in evidence['versions'].items():
        blockers = {name for name, row in data['types'].items() if 'qualification_blocker' in row}
        if rejected != blockers:
            raise ValueError(f'{version}: handwritten blocker table differs from evidence targets')
        if any(policy['versions'][version]['types'][name]['classifications']['walking'] == 'Qualified'
               for name in rejected):
            raise ValueError(f'{version}: handwritten blocker conflicts with table qualification')


def generated_docs(policy, write=False):
    check_review_enumerations(policy)
    sections = {
        'docs/contact-capability-audit.md': ('coverage', coverage_markdown(policy)),
        'docs/contact-packet-audit.md': ('packets', packets_markdown(policy)),
        'docs/contact-envelope-audit.md': ('envelopes', envelopes_markdown(policy)),
        'docs/contact-policy-consumers.md': ('manifest', manifest_markdown(policy)),
        'docs/common-entity-owners.md': ('selectors', selectors_markdown(policy)),
        'docs/common-entity-dispatch.md': ('selectors', selectors_markdown(policy)),
    }
    for filename, (tag, body) in sections.items():
        path = ROOT / filename
        old = path.read_text()
        start, end = f'<!-- contact-policy:{tag}:start -->', f'<!-- contact-policy:{tag}:end -->'
        if old.count(start) != 1 or old.count(end) != 1:
            raise ValueError(f'{filename}: expected unique generated markers')
        before, rest = old.split(start)
        _, after = rest.split(end)
        new = before + start + '\n' + body + '\n' + end + after
        if write:
            path.write_text(new)
        elif old != new:
            # Compare in memory; --check never writes committed files.
            diff = ''.join(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                               fromfile=filename, tofile='generated'))
            raise ValueError(f'stale generated docs; run --write-docs\n{diff}')
    print('contact-policy documentation sections match', file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--check', action='store_true')
    mode.add_argument('--write-docs', action='store_true')
    args = parser.parse_args()
    policy = table_export()
    if args.check or args.write_docs:
        generated_docs(policy, write=args.write_docs)
    else:
        print(json.dumps(policy, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
