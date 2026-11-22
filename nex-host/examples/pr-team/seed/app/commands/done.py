from app import store

NAME = "done"
HELP = "done ID: mark a todo done"


def run(args):
    todos = store.load()
    for t in todos:
        if str(t["id"]) == args[0]:
            t["done"] = True
            store.save(todos)
            return 0
    print(f"no todo {args[0]}")
    return 1
