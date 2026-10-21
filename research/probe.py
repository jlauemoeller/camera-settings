#!/usr/bin/env python3
"""Read-only probe of a UVC camera: standard V4L2 controls + raw extension-unit (XU) controls.

Only GET requests are issued; nothing on the device is changed.
Usage: python3 probe.py [/dev/videoN] [xu_unit_id ...]
"""
import ctypes
import errno
import fcntl
import struct
import sys

# --- ioctl number helpers (Linux asm-generic) ---
_IOC_NONE, _IOC_WRITE, _IOC_READ = 0, 1, 2


def _IOC(d, t, nr, size):
    return (d << 30) | (size << 16) | (ord(t) << 8) | nr


def _IOR(t, nr, size):
    return _IOC(_IOC_READ, t, nr, size)


def _IOWR(t, nr, size):
    return _IOC(_IOC_READ | _IOC_WRITE, t, nr, size)


# --- structs ---
class v4l2_capability(ctypes.Structure):
    _fields_ = [("driver", ctypes.c_char * 16), ("card", ctypes.c_char * 32),
                ("bus_info", ctypes.c_char * 32), ("version", ctypes.c_uint32),
                ("capabilities", ctypes.c_uint32), ("device_caps", ctypes.c_uint32),
                ("reserved", ctypes.c_uint32 * 3)]


class v4l2_query_ext_ctrl(ctypes.Structure):
    _fields_ = [("id", ctypes.c_uint32), ("type", ctypes.c_uint32), ("name", ctypes.c_char * 32),
                ("minimum", ctypes.c_int64), ("maximum", ctypes.c_int64),
                ("step", ctypes.c_uint64), ("default_value", ctypes.c_int64),
                ("flags", ctypes.c_uint32), ("elem_size", ctypes.c_uint32),
                ("elems", ctypes.c_uint32), ("nr_of_dims", ctypes.c_uint32),
                ("dims", ctypes.c_uint32 * 4), ("reserved", ctypes.c_uint32 * 32)]


class v4l2_control(ctypes.Structure):
    _fields_ = [("id", ctypes.c_uint32), ("value", ctypes.c_int32)]


class v4l2_querymenu(ctypes.Structure):
    # packed in the kernel header, but all fields are naturally aligned (44 bytes) so no _pack_ needed
    _fields_ = [("id", ctypes.c_uint32), ("index", ctypes.c_uint32),
                ("name", ctypes.c_char * 32), ("reserved", ctypes.c_uint32)]


class uvc_xu_control_query(ctypes.Structure):
    _fields_ = [("unit", ctypes.c_uint8), ("selector", ctypes.c_uint8),
                ("query", ctypes.c_uint8), ("size", ctypes.c_uint16),
                ("data", ctypes.POINTER(ctypes.c_uint8))]


VIDIOC_QUERYCAP = _IOR('V', 0, ctypes.sizeof(v4l2_capability))
VIDIOC_G_CTRL = _IOWR('V', 27, ctypes.sizeof(v4l2_control))
VIDIOC_QUERYMENU = _IOWR('V', 37, ctypes.sizeof(v4l2_querymenu))
VIDIOC_QUERY_EXT_CTRL = _IOWR('V', 103, ctypes.sizeof(v4l2_query_ext_ctrl))
UVCIOC_CTRL_QUERY = _IOWR('u', 0x21, ctypes.sizeof(uvc_xu_control_query))

V4L2_CTRL_FLAG_NEXT_CTRL = 0x80000000
V4L2_CTRL_FLAG_NEXT_COMPOUND = 0x40000000
CTRL_TYPES = {1: "int", 2: "bool", 3: "menu", 4: "button", 5: "int64", 6: "class",
              7: "string", 8: "bitmask", 9: "intmenu"}
CTRL_FLAGS = {0x1: "disabled", 0x2: "grabbed", 0x4: "read-only", 0x8: "update",
              0x10: "inactive", 0x20: "slider", 0x40: "write-only", 0x80: "volatile"}

UVC_GET_CUR, UVC_GET_MIN, UVC_GET_MAX, UVC_GET_RES = 0x81, 0x82, 0x83, 0x84
UVC_GET_LEN, UVC_GET_INFO, UVC_GET_DEF = 0x85, 0x86, 0x87

# Extension units from the BRIO's USB descriptors (unit id -> (guid, bmControls bitmap bytes))
BRIO_XUS = {
    14: ("2c49d16a-32b8-4485-3ea8-643a152362f2", [0x3f, 0x00]),
    6: ("23e49ed0-1178-4f31-ae52-d2fb8a8d3b48", [0xff, 0x6f]),
    8: ("69678ee4-410f-40db-a850-7420d7d8240e", [0x3f, 0x03]),
    9: ("1f5d4ca9-de11-4487-840d-50933c8ec8d1", [0xff, 0xff, 0x03]),
    10: ("49e40215-f434-47fe-b158-0e885023e51b", [0xba, 0x6f]),
    11: ("ffe52d21-8030-4e2c-82d9-f587d00540bd", [0x00, 0xc1, 0x03]),
    12: ("0f3f95dc-2632-4c4e-92c9-a04782f43bc8", [0x20, 0x01]),
    21: ("5a6d654c-7e35-4d4e-810d-069d15e0f79b", [0x01]),
}


def flag_str(flags):
    return ",".join(n for b, n in CTRL_FLAGS.items() if flags & b)


def list_v4l2_controls(fd):
    q = v4l2_query_ext_ctrl()
    q.id = V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND
    while True:
        try:
            fcntl.ioctl(fd, VIDIOC_QUERY_EXT_CTRL, q)
        except OSError:
            break
        name = q.name.decode(errors="replace")
        typ = CTRL_TYPES.get(q.type, str(q.type))
        if q.type == 6:
            print(f"\n  [{name}]")
        else:
            cur = "?"
            if not q.flags & 0x41:  # not disabled / write-only
                c = v4l2_control(id=q.id)
                try:
                    fcntl.ioctl(fd, VIDIOC_G_CTRL, c)
                    cur = c.value
                except OSError as e:
                    cur = f"err:{errno.errorcode.get(e.errno)}"
            line = (f"  0x{q.id:08x} {name:<34} {typ:<6} min={q.minimum} max={q.maximum} "
                    f"step={q.step} def={q.default_value} cur={cur}")
            if q.flags:
                line += f" flags={flag_str(q.flags)}"
            print(line)
            if q.type in (3, 9):
                for i in range(q.minimum, q.maximum + 1):
                    m = v4l2_querymenu(id=q.id, index=i)
                    try:
                        fcntl.ioctl(fd, VIDIOC_QUERYMENU, m)
                    except OSError:
                        continue
                    label = m.name.decode(errors="replace") if q.type == 3 else \
                        struct.unpack("<q", bytes(m.name[:8]) if isinstance(m.name, bytes) else b"")[0]
                    print(f"        {i}: {label}")
        q.id |= V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND


def xu_query(fd, unit, sel, query, size):
    buf = (ctypes.c_uint8 * max(size, 1))()
    q = uvc_xu_control_query(unit=unit, selector=sel, query=query, size=size,
                             data=ctypes.cast(buf, ctypes.POINTER(ctypes.c_uint8)))
    fcntl.ioctl(fd, UVCIOC_CTRL_QUERY, q)
    return bytes(buf[:size])


def info_str(info):
    bits = []
    if info & 1: bits.append("GET")
    if info & 2: bits.append("SET")
    if info & 4: bits.append("disabled-by-auto")
    if info & 8: bits.append("autoupdate")
    if info & 16: bits.append("async")
    return "|".join(bits) or "none"


def hexs(b):
    return b.hex(" ") if len(b) <= 32 else b[:32].hex(" ") + f" ...(+{len(b) - 32}B)"


def probe_xu(fd, unit, guid, bitmap):
    print(f"\n  XU unit {unit}  guid {{{guid}}}")
    sels = [i * 8 + b + 1 for i, byte in enumerate(bitmap) for b in range(8) if byte >> b & 1]
    for sel in sels:
        try:
            ln = struct.unpack("<H", xu_query(fd, unit, sel, UVC_GET_LEN, 2))[0]
            info = xu_query(fd, unit, sel, UVC_GET_INFO, 1)[0]
        except OSError as e:
            print(f"    sel {sel:2}: GET_LEN/INFO failed ({errno.errorcode.get(e.errno)})")
            continue
        parts = [f"len={ln:<3} info={info_str(info)}"]
        for name, qc in (("cur", UVC_GET_CUR), ("min", UVC_GET_MIN), ("max", UVC_GET_MAX),
                         ("def", UVC_GET_DEF), ("res", UVC_GET_RES)):
            if name != "cur" and not info & 2:
                continue  # range queries only meaningful for settable controls
            if name == "cur" and not info & 1:
                continue
            try:
                parts.append(f"{name}=[{hexs(xu_query(fd, unit, sel, qc, ln))}]")
            except OSError as e:
                parts.append(f"{name}=<{errno.errorcode.get(e.errno)}>")
        print(f"    sel {sel:2}: " + "  ".join(parts))


def main():
    dev = sys.argv[1] if len(sys.argv) > 1 else "/dev/video0"
    units = [int(u) for u in sys.argv[2:]] or list(BRIO_XUS)
    with open(dev, "rb", buffering=0) as f:
        fd = f.fileno()
        cap = v4l2_capability()
        fcntl.ioctl(fd, VIDIOC_QUERYCAP, cap)
        print(f"{dev}: {cap.card.decode()} driver={cap.driver.decode()} bus={cap.bus_info.decode()} "
              f"device_caps=0x{cap.device_caps:08x}")
        print("\n== Standard V4L2 controls (what the kernel uvcvideo driver maps) ==")
        list_v4l2_controls(fd)
        print("\n== Raw extension-unit controls (UVCIOC_CTRL_QUERY, GET only) ==")
        for u in units:
            guid, bitmap = BRIO_XUS[u]
            probe_xu(fd, u, guid, bitmap)


if __name__ == "__main__":
    main()
