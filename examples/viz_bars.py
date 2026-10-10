"""viz_bars.py - bars from hurricane-party's analyser, in a terminal.

The example client for the control protocol, version 1 (docs/control-api.md).
Standard library only. Run it on the PC the player runs on:

    python viz_bars.py              # until Ctrl-C
    python viz_bars.py --frames 90  # three seconds at 30 Hz, then stop

It says hello asking only for the viz stream, subscribes, lets go of the
control pipe (the subscription belongs to the viz pipe, not to it), and draws
each frame's spectrum as a row of bars. Driving LEDs is this loop with the
print swapped for your strip's library. The player never reaches out: an LED
rig elsewhere on the network is fed by a program like this one, running here.
"""
import json
import os
import socket
import struct
import sys

# Where the player listens (docs/control-api.md): a named pipe on Windows, a
# Unix domain socket on Linux and macOS.
if sys.platform == "win32":
    CONTROL = r"\\.\pipe\hurricane-party"
elif sys.platform == "darwin":
    CONTROL = os.path.expanduser("~/Library/Caches/hurricane-party.sock")
else:
    CONTROL = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "hurricane-party.sock")
# magic, timestamp_us, n_bands, depth, flags, reserved, level_peak, level_rms
HEADER = struct.Struct("<4sQBBBBBB")
RAMP = " .:-=+*#%@"


def connect(path):
    """Open a pipe or socket the player named, as an unbuffered byte stream."""
    if sys.platform == "win32":
        return open(path, "r+b", buffering=0)
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    return s.makefile("rwb", buffering=0)


def ask(pipe, msg):
    """Send one request and return its result, or stop with the reason."""
    pipe.write((json.dumps(msg) + "\n").encode())
    line = b""
    while not line.endswith(b"\n"):
        byte = pipe.read(1)
        if not byte:
            sys.exit("the player closed the pipe")
        line += byte
    reply = json.loads(line)
    if not reply.get("ok"):
        sys.exit(f"{msg['cmd']} refused: {reply.get('error')}")
    return reply["result"]


def main():
    limit = int(sys.argv[sys.argv.index("--frames") + 1]) if "--frames" in sys.argv else None
    try:
        control = connect(CONTROL)
    except OSError:
        sys.exit("couldn't reach hurricane-party: is it running?")
    with control:
        ask(control, {"id": 0, "cmd": "hello", "client": "viz_bars.py",
                      "protocol_version": 1, "want": ["viz"]})
        stream = ask(control, {"id": 1, "cmd": "subscribe_viz",
                               "bands": 32, "rate_hz": 30, "depth": "u8"})["stream"]

    shown, buf = 0, b""
    with connect(stream) as viz:
        while limit is None or shown < limit:
            chunk = viz.read(4096)
            if not chunk:
                break
            buf += chunk
            while len(buf) >= HEADER.size:
                if buf[:4] != b"HPV1":  # resync on the magic, as the doc allows
                    buf = buf[1:]
                    continue
                _, _ts, n, depth, flags, _, peak, _rms = HEADER.unpack_from(buf)
                size = HEADER.size + n * (1 if depth == 0 else 4)
                if len(buf) < size:
                    break
                body, buf = buf[HEADER.size:size], buf[size:]
                if depth == 0:
                    levels = [v / 255 for v in body]
                else:
                    levels = struct.unpack(f"<{n}f", body)
                bars = "".join(RAMP[min(len(RAMP) - 1, int(v * len(RAMP)))] for v in levels)
                beat = "*" if flags & 1 else " "
                print(f"\r[{bars}] {beat} peak {peak:3}", end="", flush=True)
                shown += 1
    print()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print()
