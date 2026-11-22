"""Todos, kept in todos.json next to where the app runs (or $TODO_FILE)."""
import json
import os


def path():
    return os.environ.get("TODO_FILE", "todos.json")


def load():
    try:
        with open(path()) as f:
            return json.load(f)
    except FileNotFoundError:
        return []


def save(todos):
    with open(path(), "w") as f:
        json.dump(todos, f, indent=2)
