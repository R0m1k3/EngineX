import os
import subprocess
import sys
import tempfile
import unittest

from notes.store import Store


def run(env_file, *args):
    env = dict(os.environ, NOTES_FILE=env_file)
    return subprocess.run([sys.executable, "-m", "notes", *args], capture_output=True, text=True, env=env)


class StoreTest(unittest.TestCase):
    def test_store(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "n.json")
            s = Store(p)
            self.assertEqual(s.add("a"), 1)
            self.assertEqual(s.add("b"), 2)
            s.done(1)
            self.assertEqual(Store(p).all(), [
                {"id": 1, "text": "a", "done": True},
                {"id": 2, "text": "b", "done": False},
            ])
            s.remove(2)
            self.assertEqual(len(Store(p).all()), 1)
            with self.assertRaises(KeyError):
                s.done(99)


class CliTest(unittest.TestCase):
    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "n.json")
            run(p, "add", "acheter du pain")
            run(p, "add", "appeler Paul")
            run(p, "done", "1")
            r = run(p, "list")
            self.assertEqual(r.stdout.splitlines(), ["[x] 1 acheter du pain", "[ ] 2 appeler Paul"])
            r = run(p, "rm", "7")
            self.assertEqual(r.returncode, 1)
            self.assertIn("introuvable", r.stderr)


if __name__ == "__main__":
    unittest.main()
