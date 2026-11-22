from app import store

NAME = "add"
HELP = "add TEXT: a new todo"


def run(args):
    todos = store.load()
    todo = {"id": max((t["id"] for t in todos), default=0) + 1, "text": " ".join(args), "done": False}
    todos.append(todo)
    store.save(todos)
    print(f"{todo['id']}: {todo['text']}")
    return 0
