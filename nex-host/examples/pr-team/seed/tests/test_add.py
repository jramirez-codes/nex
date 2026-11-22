import io
import os
import tempfile
import unittest
from contextlib import redirect_stdout

from app import store
from app.cli import main


class AddTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        os.environ["TODO_FILE"] = os.path.join(self.dir.name, "todos.json")

    def tearDown(self):
        self.dir.cleanup()

    def test_add_then_done(self):
        with redirect_stdout(io.StringIO()):
            main(["add", "buy", "milk"])
            main(["done", "1"])
        self.assertEqual(store.load(), [{"id": 1, "text": "buy milk", "done": True}])


if __name__ == "__main__":
    unittest.main()
