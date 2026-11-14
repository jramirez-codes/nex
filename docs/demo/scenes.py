"""The scripted session record.py plays, one function per website clip.

Every scene starts and ends on the same footing: a single tab, focused on a shell, so they
can be reordered or recorded on their own. `d.mark` brackets the part that becomes the clip;
make_gifs.sh names each GIF after it.
"""

CTRL_SHIFT, CTRL_ALT = "CTRL_SHIFT", "CTRL_ALT"


def close_others(d) -> None:
    """Back to one tab with one fresh shell: open a new tab, then close every pane of the
    first one (whatever is running in them). Assumes the scene's panes are all in tab 1."""
    d.key("T", CTRL_SHIFT, after=0.6)
    d.key("1", "ALT", after=0.3)
    for _ in range(d.extra_panes + 1):
        d.key("W", CTRL_SHIFT, after=0.35)
    d.extra_panes = 0
    d.wait(0.6)


def tour(d) -> None:
    """Hero: a pane per program, the header deck turning as focus moves, tabs gliding."""
    d.extra_panes = 0
    d.mark("tour", "start")
    d.wait(0.8)
    d.type("git log --oneline -8", after=1.0)
    d.key("O", CTRL_SHIFT, after=0.9)            # split right: btop gets the full-height column
    d.extra_panes += 1
    for _ in range(3):                           # zoom this pane out so all of btop fits
        d.key("minus", "CTRL", after=0.25)
    d.type("btop", after=2.2)
    d.key("Left", CTRL_ALT, after=0.6)
    d.key("E", CTRL_SHIFT, after=0.9)            # split the left pane down
    d.extra_panes += 1
    d.type("nvim src/main.rs", after=1.8)
    # Walk focus around the panes: the tab header's pane-icon deck turns with it.
    d.key("Right", CTRL_ALT, after=1.1)
    d.key("Left", CTRL_ALT, after=1.1)
    d.key("Up", CTRL_ALT, after=1.1)
    d.key("Right", CTRL_ALT, after=1.2)
    # A second tab, then back and forth: the glass pill glides between them.
    d.key("T", CTRL_SHIFT, after=1.0)
    d.type("tree -L 1 --dirsfirst src", after=1.6)
    d.key("1", "ALT", after=1.4)
    d.key("2", "ALT", after=1.4)
    d.key("1", "ALT", after=1.6)
    d.mark("tour", "end")
    d.key("2", "ALT", after=0.4)                  # close the second tab first
    d.key("W", CTRL_SHIFT, after=0.6)
    close_others(d)


def splits(d) -> None:
    """Spotlight 1: panes slide out of the seam as they split, zoom, resize and equalize,
    then close (collapsing and fading) back down to one."""
    d.extra_panes = 0
    d.mark("splits", "start")
    d.wait(0.6)
    d.type("git log --oneline --graph -12", after=0.8)
    d.key("O", CTRL_SHIFT, after=0.9)
    d.extra_panes += 1
    d.type("ls -la src", after=0.9)
    d.key("E", CTRL_SHIFT, after=0.9)
    d.extra_panes += 1
    d.type("nvim src/app/layout.rs", after=1.6)
    d.key("Return", CTRL_SHIFT, after=1.6)       # zoom it
    d.key("Return", CTRL_SHIFT, after=1.2)       # and back
    for _ in range(3):
        d.key("Left", "CTRL_SHIFT_ALT", after=0.25)  # grow the right column
    d.wait(0.8)
    d.key("equal", "CTRL_SHIFT_ALT", after=1.2)  # equalize
    d.key("Up", CTRL_ALT, after=0.9)             # focus fades across to ls...
    d.key("Left", CTRL_ALT, after=0.9)           # ...and to git log
    d.key("Right", CTRL_ALT, after=0.6)
    d.key("W", CTRL_SHIFT, after=1.0)            # close ls: nvim grows into its space
    d.key("W", CTRL_SHIFT, after=1.4)            # close nvim: back to one pane
    d.extra_panes -= 2
    d.mark("splits", "end")
    close_others(d)


def explorer(d) -> None:
    """Spotlight 2: Ctrl+E file explorer, browse, multi-select, grid view, shell in a folder."""
    d.extra_panes = 0
    d.mark("explorer", "start")
    d.wait(0.6)
    d.type("git status --short", after=0.8)
    d.key("E", "CTRL", after=1.4)                # explorer splits off, at the shell's folder
    d.extra_panes += 1
    for _ in range(4):                           # assets, bench, docs, site, src
        d.key("Down", after=0.3)
    d.key("Return", after=1.0)                   # into src/
    d.key("Down", after=0.3)
    d.key("space", after=0.3)
    d.key("Down", after=0.3)
    d.key("space", after=0.3)
    d.key("Down", after=0.3)
    d.key("space", after=1.0)                    # three files marked
    d.key("G", "CTRL", after=1.6)                # grid view
    d.key("G", "CTRL", after=1.0)                # list view
    d.key("E", CTRL_SHIFT, after=1.0)            # split a shell off the explorer, in src/
    d.extra_panes += 1
    d.type("ls --color=always", after=1.8)
    d.mark("explorer", "end")
    close_others(d)


def search(d) -> None:
    """Spotlight 3: scrollback search highlighting every match, stepping through them."""
    d.extra_panes = 0
    d.mark("search", "start")
    d.wait(0.6)
    d.type("git log --oneline -60", after=1.0)
    d.key("F", CTRL_SHIFT, after=0.6)
    d.type("pane", enter=False, after=1.0)
    for _ in range(4):
        d.key("Return", after=0.7)
    d.wait(0.6)
    d.key("Escape", after=0.8)
    d.mark("search", "end")
    close_others(d)


SCENES = [tour, splits, explorer, search]
