"""Dump Xilinx 7-series/UltraScale configuration packets (BSPI, WBSTAR, TIMER, COR, IDCODE, CMD ...)
up to the first large FDRI write, for each sync word. Usage: python -I tools/bitstream_packets.py <component_A.bin> [...]"""
import struct, sys
REG = {0:"CRC",1:"FAR",2:"FDRI",3:"FDRO",4:"CMD",5:"CTL0",6:"MASK",7:"STAT",8:"LOUT",9:"COR0",
       10:"MFWR",11:"CBC",12:"IDCODE",13:"AXSS",14:"COR1",16:"WBSTAR",17:"TIMER",19:"RBCRC_SW",
       22:"BOOTSTS",24:"CTL1",31:"BSPI"}
CMD = {0:"NULL",1:"WCFG",2:"MFW",3:"LFRM",4:"RCFG",5:"START",7:"RCRC",8:"AGHIGH",10:"GRESTORE",
       11:"SHUTDOWN",13:"DESYNC",15:"IPROG",16:"CRCC",17:"LTIMER",18:"BSPI_READ",19:"FALL_EDGE"}
for path in sys.argv[1:]:
    d = open(path, "rb").read()
    print(f"== {path} ({len(d)} B) header[0:8]={d[:8].hex()}")
    start = 0
    nsync = 0
    while True:
        off = d.find(b"\xaa\x99\x55\x66", start)
        if off < 0 or nsync > 3: break
        nsync += 1
        print(f"  SYNC at {off}")
        p = off + 4; last = None; n = 0
        while p + 4 <= len(d) and n < 120:
            (w,) = struct.unpack_from(">I", d, p); p += 4
            t = w >> 29
            if w == 0x20000000: continue  # NOOP
            if t == 1:
                op = (w >> 27) & 3; reg = (w >> 13) & 0x3FFF; cnt = w & 0x7FF; last = reg
                if op == 2 and cnt:
                    vals = struct.unpack_from(f">{min(cnt,4)}I", d, p)
                    s = ", ".join(f"0x{v:08x}" + (f"({CMD.get(v,'?')})" if reg == 4 else "") for v in vals)
                    print(f"    W {REG.get(reg,reg)} = {s}" + (" ..." if cnt > 4 else ""))
                    p += 4 * cnt
                elif op == 2:
                    pass
                else:
                    print(f"    op{op} {REG.get(reg,reg)} cnt={cnt}")
            elif t == 2:
                cnt = w & 0x7FFFFFF
                print(f"    type2 to {REG.get(last,last)}: {cnt} words ({cnt*4} B) -> skip")
                p += 4 * cnt
                n += 1
                continue
            n += 1
        start = off + 4
        # find later sync words only after this config chunk
        start = p
