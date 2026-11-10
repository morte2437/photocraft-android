#!/usr/bin/env python3
"""Run the configured GPU group on lavapipe without silently losing comparisons."""
import glob
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib


def gpu_filter(config):
    filters = [row['filter'] for row in config['profile']['default']['overrides']
               if row.get('test-group') == 'gpu']
    if not filters:
        raise ValueError('nextest GPU group has no filters')
    return ' | '.join(f'({value})' for value in filters)


def validate_log(lines):
    vulkan = False
    rejected = []
    for line in lines:
        vulkan |= 'GPU parity backend: Vulkan' in line
        # Authored skips, not nextest's summary of ignored/filtered-out tests.
        if re.search(r'\bskipping\b|\bno (?:GPU )?adapter\b', line):
            rejected.append(line.rstrip())
        if 'SKIP ' in line and not (
            ('SKIP gpu timing:' in line and 'is a software adapter' in line)
            or 'SKIP the comparison: this GPU finishes' in line
            or 'SKIP the sum: this refresh still fit one command buffer' in line
        ):
            rejected.append(line.rstrip())
    if not vulkan:
        rejected.append('GPU parity tests did not report the Vulkan backend')
    return rejected


def main():
    root = Path(__file__).resolve().parents[1]
    with (root / '.config/nextest.toml').open('rb') as source:
        selected = gpu_filter(tomllib.load(source))
    icds = sorted(glob.glob('/usr/share/vulkan/icd.d/lvp_icd*.json'))
    if not icds:
        raise RuntimeError('Install mesa-vulkan-drivers and libvulkan1 before this check')
    env = dict(os.environ, WGPU_BACKEND='vulkan', VK_ICD_FILENAMES=os.pathsep.join(icds))
    command = ['cargo', 'nextest', 'run', '--profile', 'ci', '--workspace', '--locked',
               '--features', 'heif', '--lib', '--bins', '--tests', '-E', selected,
               '--success-output', 'immediate', '--color', 'never']
    # Stream output and keep it on disk, rather than buffering every successful test in memory.
    with tempfile.TemporaryFile(mode='w+t') as log:
        with subprocess.Popen(command, cwd=root, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True) as process:
            for line in process.stdout:
                sys.stdout.write(line)
                log.write(line)
            result = process.wait()
        log.seek(0)
        errors = validate_log(log)
    for error in errors:
        print(f'::error::{error}', file=sys.stderr)
    return result if result else int(bool(errors))


if __name__ == '__main__':
    sys.exit(main())
