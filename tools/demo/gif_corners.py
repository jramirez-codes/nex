"""Make the rounded corners of a recorded window transparent in an ffmpeg-made GIF.

ffmpeg's GIF encoder saves size by writing each frame as only the rectangle that changed, with
unchanged pixels marked transparent. A palette with a real transparent color turns that off and
every frame goes out whole (20x the size). So the GIF is made opaque, with the corners painted
one constant color (make_gifs.py does that), and only the first frame is rewritten here: its
corner pixels move to a palette index that frame doesn't otherwise use, flagged transparent in
its Graphic Control Extension. The corners never change, so later frames leave them as they are.
"""

from pathlib import Path


def _sub_blocks(data: bytes, i: int) -> tuple[bytes, int]:
    """Concatenated sub-block payloads starting at i, and the index after the terminator."""
    out = bytearray()
    while data[i]:
        out += data[i + 1:i + 1 + data[i]]
        i += 1 + data[i]
    return bytes(out), i + 1


def _pack_sub_blocks(payload: bytes) -> bytes:
    out = bytearray()
    for i in range(0, len(payload), 255):
        chunk = payload[i:i + 255]
        out += bytes([len(chunk)]) + chunk
    return bytes(out + b"\0")


def _lzw_decode(data: bytes, min_size: int, count: int) -> bytearray:
    clear, end = 1 << min_size, (1 << min_size) + 1
    table: list[bytes] = []
    out, prev = bytearray(), None
    size, bits, nbits = min_size + 1, 0, 0
    for byte in data:
        bits |= byte << nbits
        nbits += 8
        while nbits >= size:
            code = bits & ((1 << size) - 1)
            bits >>= size
            nbits -= size
            if code == clear:
                table = [bytes([c]) for c in range(clear)] + [b"", b""]
                size, prev = min_size + 1, None
                continue
            if code == end:
                return out[:count]
            if prev is None:
                entry = table[code]
            elif code < len(table):
                entry = table[code]
                table.append(prev + entry[:1])
            else:
                entry = prev + prev[:1]
                table.append(entry)
            out += entry
            prev = entry
            if len(table) == 1 << size and size < 12:
                size += 1
    return out[:count]


def _lzw_encode(pixels: bytes, min_size: int) -> bytes:
    clear, end = 1 << min_size, (1 << min_size) + 1
    out, bits, nbits = bytearray(), 0, 0

    def emit(code: int, size: int) -> None:
        nonlocal bits, nbits
        bits |= code << nbits
        nbits += size
        while nbits >= 8:
            out.append(bits & 0xFF)
            bits >>= 8
            nbits -= 8

    def reset() -> tuple[dict, int, int]:
        return {}, end + 1, min_size + 1

    table, next_code, size = reset()
    emit(clear, size)
    prefix = pixels[0]
    for c in pixels[1:]:
        key = (prefix, c)
        if key in table:
            prefix = table[key]
            continue
        emit(prefix, size)
        if next_code == 4096:
            emit(clear, size)
            table, next_code, size = reset()
        else:
            table[key] = next_code
            next_code += 1
            if next_code > 1 << size and size < 12:
                size += 1
        prefix = c
    emit(prefix, size)
    emit(end, size)
    if nbits:
        out.append(bits & 0xFF)
    return bytes(out)


def _outside(x: int, y: int, w: int, h: int, r: float) -> bool:
    """Whether pixel (x, y) lies outside a w x h rectangle with corners rounded to radius r."""
    dx = max(r - x - 0.5, x + 0.5 - (w - r), 0)
    dy = max(r - y - 0.5, y + 0.5 - (h - r), 0)
    return dx * dx + dy * dy > r * r


def clear_corners(path: Path, radius: float) -> None:
    data = bytearray(path.read_bytes())
    w, h = int.from_bytes(data[6:8], "little"), int.from_bytes(data[8:10], "little")
    i = 13 + (3 << ((data[10] & 7) + 1) if data[10] & 0x80 else 0)
    gce = None
    while data[i] == 0x21:  # extensions before the first image
        if data[i + 1] == 0xF9:
            gce = i
        _, i = _sub_blocks(data, i + 2)
    assert data[i] == 0x2C, "expected the first image descriptor"
    desc = i
    fx, fy, fw, fh = (int.from_bytes(data[desc + k:desc + k + 2], "little") for k in (1, 3, 5, 7))
    flags = data[desc + 9]
    assert (fx, fy, fw, fh) == (0, 0, w, h), "first frame must cover the whole canvas"
    assert not flags & 0x40, "first frame must not be interlaced"
    lzw = desc + 10 + (3 << ((flags & 7) + 1) if flags & 0x80 else 0)  # past any local palette
    min_size = data[lzw]
    payload, after = _sub_blocks(data, lzw + 1)
    pixels = _lzw_decode(payload, min_size, w * h)

    r = radius
    corners = [y * w + x for y in range(h) for x in range(w)
               if (x < r or x >= w - r) and (y < r or y >= h - r) and _outside(x, y, w, h, r)]
    corner_set = set(corners)
    used = {p for k, p in enumerate(pixels) if k not in corner_set}
    free = [c for c in range(1 << min_size) if c not in used]
    assert free, "first frame uses every palette color; no index left for transparency"
    trans = free[-1]
    for k in corners:
        pixels[k] = trans

    image = bytes([min_size]) + _pack_sub_blocks(_lzw_encode(bytes(pixels), min_size))
    data[lzw:after] = image
    if gce is None:  # disposal 1 (keep), no delay change needed for a lone first frame
        data[desc:desc] = bytes([0x21, 0xF9, 4, 0x05, 0, 0, trans, 0])
    else:
        assert data[gce + 3] & 0x1C in (0, 0x04), "first frame must not be disposed"
        data[gce + 3] |= 0x01
        data[gce + 6] = trans
    path.write_bytes(bytes(data))
