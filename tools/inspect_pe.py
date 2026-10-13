"""Read Windows subsystem and DLL imports directly from the built PE file."""
import json, struct, sys
from pathlib import Path

data = Path(sys.argv[1]).read_bytes()
def u16(p): return struct.unpack_from('<H', data, p)[0]
def u32(p): return struct.unpack_from('<I', data, p)[0]
pe = u32(0x3c)
assert data[pe:pe+4] == b'PE\0\0'
optional = pe + 24
magic = u16(optional)
sections = []
section = optional + u16(pe + 20)
for i in range(u16(pe+6)):
    p = section + i*40
    sections.append((u32(p+12), max(u32(p+8),u32(p+16)), u32(p+20)))
def offset(rva):
    for base, size, raw in sections:
        if base <= rva < base+size: return raw+rva-base
    raise ValueError(f'Unmapped RVA {rva:x}')
directories = optional + (112 if magic == 0x20b else 96)
imports = []
rva = u32(directories+8)
if rva:
    p = offset(rva)
    while any(data[p:p+20]):
        name = offset(u32(p+12))
        imports.append(data[name:data.index(0,name)].decode('ascii'))
        p += 20
print(json.dumps({'path':str(Path(sys.argv[1]).resolve()), 'machine':hex(u16(pe+4)), 'subsystem':u16(optional+68), 'subsystem_name':{2:'Windows GUI',3:'Windows Console'}.get(u16(optional+68)), 'dlls':sorted(imports)},indent=2))
