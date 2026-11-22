from app import store

NAME = "ls"
HELP = "ls: all todos"


def run(args):
    for t in store.load():
        print(f"[{'x' if t['done'] else ' '}] {t['id']}: {t['text']}")
    return 0
