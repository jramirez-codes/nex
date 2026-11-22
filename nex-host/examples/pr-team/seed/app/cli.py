"""`python3 -m app.cli COMMAND [ARGS]`. Each command is a module in app/commands/ with a
NAME, a HELP line and run(args) -> int; new ones are found on their own."""
import importlib
import pkgutil
import sys

from app import commands


def available():
    found = {}
    for info in pkgutil.iter_modules(commands.__path__):
        mod = importlib.import_module(f"app.commands.{info.name}")
        found[mod.NAME] = mod
    return found


def main(argv):
    cmds = available()
    if not argv or argv[0] not in cmds:
        for name, mod in sorted(cmds.items()):
            print(f"{name:<10} {mod.HELP}")
        return 2
    return cmds[argv[0]].run(argv[1:])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
