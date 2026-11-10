import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('gpu_check', Path(__file__).with_name('ci-gpu-lavapipe.py'))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class LogChecks(unittest.TestCase):
    def test_requires_vulkan_evidence(self):
        self.assertTrue(check.validate_log(['GPU parity backend: Gl']))
        self.assertTrue(check.validate_log([]))

    def test_success_and_nextest_summary(self):
        self.assertFalse(check.validate_log(['GPU parity backend: Vulkan (llvmpipe, Cpu)',
                                            'Summary 100 tests run: 100 passed, 3 skipped']))

    def test_rejects_missing_comparisons(self):
        for message in ['skipping GPU parity tests: no device', 'skipping: unsupported format',
                        'no GPU adapter', 'SKIP gpu timing: no adapter',
                        'SKIP gpu timing: no device with timestamp queries', 'SKIP unknown test']:
            with self.subTest(message=message):
                self.assertTrue(check.validate_log(['GPU parity backend: Vulkan', message]))

    def test_allows_only_known_timing_exceptions(self):
        self.assertFalse(check.validate_log(['GPU parity backend: Vulkan',
            'SKIP gpu timing: llvmpipe is a software adapter, which cannot show a GPU/CPU split',
            'SKIP the comparison: this GPU finishes a 6 MP composite in 1 ms against 2 ms of encoding',
            'SKIP the sum: this refresh still fit one command buffer']))

    def test_selects_all_gpu_filters_from_config(self):
        config = {'profile': {'default': {'overrides': [
            {'filter': 'package(a)', 'test-group': 'gpu'},
            {'filter': 'package(b)', 'test-group': 'cpu'},
            {'filter': 'test(c)', 'test-group': 'gpu'}]}}}
        self.assertEqual(check.gpu_filter(config), '(package(a)) | (test(c))')
        config['profile']['default']['overrides'] = []
        with self.assertRaises(ValueError):
            check.gpu_filter(config)


if __name__ == '__main__':
    unittest.main()
