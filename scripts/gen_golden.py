#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""gen_golden.py — 用 Python 版 wd_pak_tool 生成 Rust 重写的黄金测试向量。

产物全部位于 wd-pak-rust/tests/golden/，幂等可重跑：
  - extract_files.sha256        两个真实 pak 全量解压清单（含 manifest 本身）
  - lzss_decompress_cases.json  解压向量索引（30 小 + 15 大 + 3 合成初始窗口流）
  - decomp/<id>.bin/.out        小文件存储流 / 期望解压输出
  - lzss_compress_cases.json    合成边界压缩向量（hex 内嵌 / 大输入落盘）
  - lzss_compress_files.json    etc.pak 抽样 40 文件压缩向量
  - comp/                       压缩向量输入/输出落盘文件
  - layout/                     mini pak 布局向量（no-compress + compress）及输入目录
  - replace/                    replace 输出哈希向量
  - golden_index.json           总索引（含 Python 版行为细节 notes）

性能约束：不对 218MB 数据做全量压缩；压缩向量仅抽样 + 合成边界。
删除约束（AGENTS.md）：仅 rmtree 本脚本受管的产物目录与 mkdtemp 临时目录。
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]          # /root/wen1.60
TOOL = ROOT / 'wd_pak_tool.py'
GOLDEN = ROOT / 'wd-pak-rust' / 'tests' / 'golden'
TMP_BASE = ROOT / 'wd-pak-rust' / '.golden_tmp'
ETC_DIR = ROOT / 'etc.pak'                          # etc.pak 解包目录（已存在）
PAKS = {
    'etc.pak': ROOT / 'release' / 'etc.pak',
    'lib_gs32.pak': ROOT / 'release' / 'lib_gs32.pak',
}

sys.path.insert(0, str(ROOT))
import wd_pak_tool as wpt                            # noqa: E402

HEX_INLINE_MAX = 4096        # 输入/输出 ≤4KB 时 hex 内嵌 JSON，否则落盘
COMP_FILE_LIMIT = 256 * 1024  # 抽样压缩向量：≤256KB 存输入+输出，更大只记哈希

# Python 版行为细节（供 Rust 实现者对照，同步写入 golden_index.json）
NOTES = [
    'lzss_compress(b"") == b""（无 flag 字节）；lzss_decompress(b"", 0) == b""',
    '解压初始环形缓冲 = 4078 个 0x20(空格) + 18 个 0x00，起始写指针 r0=4078；'
    '匹配的 i 直接作环形下标用，复制时 i 逐字节 +1 跨 4096 环绕',
    '解压 flag 机制：flags = data[pos] | 0xFF00，(flags & 0x100)==0 时取新 flag 字节；'
    'bit=1 字面量、bit=0 匹配；输出达到 out_size 立即停止并忽略流中剩余字节',
    '匹配解码：i |= (j & 0xF0) << 4；长度 = (j & 0x0F) + 2 + 1，即 3..18',
    '压缩器匹配接受条件：1 <= d <= 4078 且 d <= i（d = (r - cand) & 4095），'
    '因此 Python 压缩流永不引用初始窗口；哈希链最多探测 128 个候选；MIN_MATCH=3',
    '压缩器哈希链插入条件：字面量处 i+2 < n、匹配内逐字节 i+k+2 < n，即末尾 2 字节不插链',
    '压缩 flag 字节先写占位 0，凑满 8 项或输入结束时回填；每 bit 对应一组（字面量 bit=1）',
    '两个真实 pak 全部条目 attrib=0（LZSS），无 RAW_BIT 条目；etc.pak 无目录，'
    'lib_gs32.pak 含 519 个目录；抽样 800 个小文件未发现初始窗口引用与流尾部剩余',
    'read_file 对 RAW_BIT 条目返回 stored[:org]（可能截断）；目录条目 size=org=64*子条目数，'
    '数据为子目录表原样字节',
    'pack 排序：无 manifest 时按名字的 UTF-8(surrogateescape) 字节序；有 manifest 时 order '
    '索引优先，不在 order 的条目（键 1<<30）排同层最后；文件 mtime 默认 0，manifest 可覆盖'
    '（校验为 0..0xFFFFFFFF 的 int）；目录条目 mtime 恒为 0；_pak_manifest.json 自动跳过',
    'pak 内文件名为 latin1 单字节编码（磁盘 UTF-8 名需转 latin1 字节），最长 43 字节，'
    '禁止 / \\\\ 与 NUL；pack preflight 会拒绝同层大小写冲突',
    'replace 为整包重写：未修改条目按原 offset/size 从源 pak 流式拷贝（字节原样），'
    '全部条目 mtime 保留原值；被替换条目 mtime 也保留替换前的旧值',
    'Header 16B：magic=0x11223344, rootDirSize, unused1=0, unused2=0（pack 恒写 0）',
    '布局：header(offset 0) -> 根目录表(offset 16) -> 各条目数据按目录表顺序深度优先排列；'
    '每个目录节点的数据区开头是它的子目录表',
]


def log(msg):
    print(f'[gen_golden] {msg}', flush=True)


def sha256_bytes(b):
    return hashlib.sha256(b).hexdigest()


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, 'rb') as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def write_bytes(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'wb') as fh:
        fh.write(data)


def write_json(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'w', encoding='utf-8') as fh:
        json.dump(obj, fh, indent=1, ensure_ascii=False)


def rmtree_owned(p, why):
    """仅删除本脚本受管的产物/临时目录；带路径守卫与日志（AGENTS.md 删除约束）。"""
    rp = p.resolve()
    guard = (ROOT / 'wd-pak-rust').resolve()
    if rp == guard or not str(rp).startswith(str(guard) + os.sep):
        raise RuntimeError(f'拒绝删除非受管路径: {p}')
    if rp.exists():
        log(f'清理受管目录 {rp}（{why}）')
        shutil.rmtree(str(rp))


@contextmanager
def tmp_dir(tag):
    """mkdtemp 临时目录，结束仅删除本次创建的该目录。"""
    TMP_BASE.mkdir(parents=True, exist_ok=True)
    d = Path(tempfile.mkdtemp(prefix=f'{tag}-', dir=str(TMP_BASE)))
    log(f'临时目录 {d}')
    try:
        yield d
    finally:
        shutil.rmtree(str(d))
        try:
            TMP_BASE.rmdir()   # 空壳顺手移除，非空（并发）则保留
        except OSError:
            pass


def run_cli(args, timeout=900):
    """调用 Python 版 CLI；失败抛异常。"""
    cmd = [sys.executable, str(TOOL)] + [str(a) for a in args]
    log('CLI: ' + ' '.join(cmd[1:]))
    r = subprocess.run(cmd, cwd=str(ROOT), capture_output=True,
                       text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f'CLI 失败 rc={r.returncode}: '
                           f'{" ".join(cmd[1:])}\nstderr: {r.stderr}')
    return r


def walk_files(pak):
    """(rel, node) 列表，仅文件条目（walk 序 = pak 目录表序）。"""
    out = []
    for rel, e in pak.walk():
        if not (e['attrib'] & wpt.DIR_BIT):
            out.append((rel, e))
    return out


# --------------------------------------------------------------------------
# 1. 布局向量（先生成：no-compress pak 是 RAW_BIT 解压向量的来源）
# --------------------------------------------------------------------------
MINI_FILES = [
    'task_score.list',            # 0 字节（真实空文件）
    'luck_magic_special.list',    # 8B
    'basic_book.list',            # 16B
    'amend.list',
    '100heis.list',               # 7291B，与 replace 向量同款目标
    'account_question.list',
    'zuanshi -test.list',         # 特殊 ASCII 名（含空格）
    'ac_combat_answer.c',         # .c 源码
    'Font1.ttf',                  # 小 ttf
    'Font2.ttf',
    'zzz10001.list',              # ~60KB
    'libao007.list',              # ~228KB
    'boss_template.list',         # 261917B，接近 256KB 边界
    'aiwo_zhonghua.list',
    'aiwo_zhonghua_test.list',
]
CAFE_NAME = 'café.list'           # latin1 高位字节名（0xE9），磁盘上为 UTF-8
CAFE_CONTENT = b'golden-layout-cafe-' * 100


def gen_layout():
    mini_dir = GOLDEN / 'layout' / 'etc_mini'
    rmtree_owned(mini_dir, '重建布局输入目录')
    mini_dir.mkdir(parents=True)

    # 从 release/etc.pak 提取真实 mtime 与 walk 顺序，自建 manifest（测 order 保序 + mtime 应用）
    pak = wpt.Pak(str(PAKS['etc.pak']))
    try:
        info, order_all = {}, []
        for rel, e in pak.walk():
            if not (e['attrib'] & wpt.DIR_BIT):
                info[rel] = e
                order_all.append(rel)
    finally:
        pak.close()
    missing = [n for n in MINI_FILES if n not in info]
    if missing:
        raise RuntimeError(f'布局清单文件在 etc.pak 中缺失: {missing}')
    for n in MINI_FILES:
        shutil.copyfile(str(ETC_DIR / n), str(mini_dir / n))
    write_bytes(mini_dir / CAFE_NAME, CAFE_CONTENT)
    subset = set(MINI_FILES)
    write_json(mini_dir / '_pak_manifest.json', {
        'pak': 'etc_mini',
        'order': [rel for rel in order_all if rel in subset],  # café.list 不在 order → 排最后
        'files': {rel: {'attrib': info[rel]['attrib'], 'mtime': info[rel]['mtime'],
                        'stored': info[rel]['size'], 'orig': info[rel]['org']}
                  for rel in order_all if rel in subset},
    })

    nc_pak = GOLDEN / 'layout' / 'etc_mini_nocompress.pak'
    cp_pak = GOLDEN / 'layout' / 'etc_mini_compress.pak'
    run_cli(['pack', mini_dir, nc_pak, '--no-compress', '--jobs', 1])
    run_cli(['pack', mini_dir, cp_pak, '--jobs', 1])

    # 验证：两个 pak 读回，16 个条目逐一与源字节一致
    expect = {n: (ETC_DIR / n).read_bytes() for n in MINI_FILES}
    expect[CAFE_NAME] = CAFE_CONTENT
    for pp in (nc_pak, cp_pak):
        p = wpt.Pak(str(pp))
        try:
            got = dict(walk_files(p))
            assert set(got) == set(expect), f'{pp.name} 条目集不符: {set(got) ^ set(expect)}'
            for rel, node in got.items():
                assert p.read_file(node) == expect[rel], f'{pp.name}:{rel} 内容不符'
        finally:
            p.close()
    log(f'布局 pak 验证通过（各 {len(expect)} 条目）')

    # 实际打包读取顺序（复刻 _collect_files 的 UTF-8 字节序 + manifest 跳过）
    collect_order = sorted(os.listdir(mini_dir),
                           key=lambda x: x.encode('utf-8', 'surrogateescape'))
    collect_order = [n for n in collect_order if n != wpt.MANIFEST_NAME]
    write_json(GOLDEN / 'layout' / 'layout.json', {
        'input_dir': 'layout/etc_mini',
        'collect_order': collect_order,          # 磁盘枚举序（Rust 复现输入用）
        'manifest_order': [rel for rel in order_all if rel in subset],
        'input_files': [{'name': n, 'len': len(expect[n]),
                         'sha256': sha256_bytes(expect[n])} for n in sorted(expect)],
        'nocompress': {'pak': 'layout/etc_mini_nocompress.pak',
                       'sha256': sha256_file(nc_pak), 'size': nc_pak.stat().st_size},
        'compress': {'pak': 'layout/etc_mini_compress.pak',
                     'sha256': sha256_file(cp_pak), 'size': cp_pak.stat().st_size},
    })
    return {'input_files': len(expect), 'paks': [nc_pak.name, cp_pak.name]}


# --------------------------------------------------------------------------
# 2. 全量解压清单
# --------------------------------------------------------------------------
def gen_extract():
    lines, stats = [], {}
    for pakname, pakpath in PAKS.items():
        with tmp_dir('extract') as d:
            run_cli(['extract', pakpath, d])
            mf = json.loads((d / '_pak_manifest.json').read_text(encoding='utf-8'))
            n, total = 0, 0
            for rel in mf['order']:
                if rel not in mf['files']:        # 目录条目只建目录，不出清单行
                    continue
                lines.append(f'{sha256_file(d / rel)}  {pakname}:{rel}')
                n += 1
                total += mf['files'][rel]['orig']
            lines.append(f'{sha256_file(d / "_pak_manifest.json")}  '
                         f'{pakname}:_pak_manifest.json')
            stats[pakname] = {'files': n, 'org_bytes': total}
            log(f'{pakname}: 清单 {n} 文件 + manifest，org 共 {total / 1e6:.1f} MB')
    write_bytes(GOLDEN / 'extract_files.sha256', ('\n'.join(lines) + '\n').encode())
    return {'lines': len(lines), 'paks': stats}


# --------------------------------------------------------------------------
# 3. LZSS 解压向量
# --------------------------------------------------------------------------
# 合成流：手工构造引用初始窗口（真实 pak 抽样 800 例均未出现，官方流无此形态）
SYNTHETIC_STREAMS = [
    ('init-window-space-ref',
     b'\x00\x00\x0f', 18, b' ' * 18),                 # 匹配 i=0 len=18 → 初始空格区
    ('init-window-zero-ref',
     b'\x00\xee\xff', 18, b'\x00' * 18),              # 匹配 i=4078 len=18 → 初始 0x00 区
    ('literal+init-ref+trailing-bytes',
     b'\x01\x58\x00\x0f\xaa\xbb\xcc', 19, b'X' + b' ' * 18),  # 达标即停，尾部 3 字节被忽略
]


def pick_diversity(small_by_pak, want, taken):
    """按尺寸桶优先级确定性取样凑满 want 条（桶内 (org, rel) 升序），跳过已选。"""
    buckets = [(0, 1), (1, 512), (512, 4096), (4096, 65537)]  # ≤64KB，含 0 字节桶
    picked = []
    for lo, hi in buckets:
        for pakname in ('etc.pak', 'lib_gs32.pak'):
            cands = sorted(((e['org'], rel, e) for rel, e in small_by_pak[pakname]
                            if lo <= e['org'] < hi and (pakname, rel) not in taken),
                           key=lambda x: (x[0], x[1]))
            for _, rel, e in cands:
                if len(picked) == want:
                    return picked
                picked.append((pakname, rel, e))
                taken.add((pakname, rel))
    return picked


def gen_decompress(nc_pak_path):
    cases_small, cases_large = [], []
    taken = set()

    # a) RAW_BIT 条目 ×4：来自 no-compress 布局 pak（真实 pak 中不存在 RAW 条目）
    p = wpt.Pak(str(nc_pak_path))
    try:
        for name in ('task_score.list', '100heis.list', 'zuanshi -test.list', CAFE_NAME):
            node = p.find(name)
            assert node is not None, f'no-compress pak 缺 {name}'
            assert node['attrib'] & wpt.RAW_BIT, f'{name} 不是 RAW 条目'
            cases_small.append(('[nocompress pak]', name, node))
            taken.add(('etc_mini_nocompress.pak', name))
    finally:
        p.close()

    # b) lib 目录邻居 ×4：walk 序中目录条目之后的首个 ≤64KB 文件（不同父目录）
    lib = wpt.Pak(str(PAKS['lib_gs32.pak']))
    etc = wpt.Pak(str(PAKS['etc.pak']))
    try:
        neighbors, after_dir = [], False
        for rel, e in lib.walk():
            if e['attrib'] & wpt.DIR_BIT:
                after_dir = True
                continue
            if after_dir and e['org'] <= 65536:
                neighbors.append((rel, e))
                after_dir = False
                if len(neighbors) == 4:
                    break
        for rel, e in neighbors:
            cases_small.append(('lib_gs32.pak', rel, e))
            taken.add(('lib_gs32.pak', rel))

        # c) 尺寸桶多样性：etc + lib 真实 LZSS 小文件
        small_by_pak = {
            'etc.pak': [(r, e) for r, e in walk_files(etc) if e['org'] <= 65536],
            'lib_gs32.pak': [(r, e) for r, e in walk_files(lib) if e['org'] <= 65536],
        }
        for pakname, rel, e in pick_diversity(small_by_pak, 19, taken):
            cases_small.append((pakname, rel, e))
            taken.add((pakname, rel))

        # d) 大文件 ×15：etc top8 + lib top7（(-org, rel) 序）
        for pakname, pk, q in (('etc.pak', etc, 8), ('lib_gs32.pak', lib, 7)):
            for rel, e in sorted(walk_files(pk), key=lambda x: (-x[1]['org'], x[0]))[:q]:
                out = pk.read_file(e)
                assert len(out) == e['org']
                cases_large.append({'id': len(cases_large) + 1, 'pak': pakname,
                                    'inner': rel, 'org': e['org'],
                                    'offset': e['offset'], 'size': e['size'],
                                    'expect_sha256': sha256_bytes(out)})
    finally:
        lib.close()
        etc.close()

    # 写 small：真实条目 stored/out 落盘；合成流自证后同样落盘
    small_json = []
    real_cases = [(pak, rel, e) for pak, rel, e in cases_small]
    idx = 0
    for pakname, rel, e in real_cases:
        idx += 1
        stored = None
        if pakname == '[nocompress pak]':
            pk = wpt.Pak(str(nc_pak_path))
            try:
                stored = pk.read_stored(pk.find(rel))
                out = pk.read_file(pk.find(rel))
            finally:
                pk.close()
        else:
            pk = wpt.Pak(str(PAKS[pakname]))
            try:
                stored = pk.read_stored(pk.find(rel))
                out = pk.read_file(e)
            finally:
                pk.close()
        assert len(out) == e['org'], f'{rel} 解压长度不符'
        write_bytes(GOLDEN / 'decomp' / f'{idx:03d}.bin', stored)
        write_bytes(GOLDEN / 'decomp' / f'{idx:03d}.out', out)
        small_json.append({'id': idx, 'pak': 'etc_mini_nocompress.pak'
                           if pakname == '[nocompress pak]' else pakname,
                           'inner': rel, 'org': e['org'],
                           'stored_file': f'decomp/{idx:03d}.bin',
                           'out_file': f'decomp/{idx:03d}.out',
                           'stored_sha256': sha256_bytes(stored),
                           'out_sha256': sha256_bytes(out)})
    for desc, stream, org, expect in SYNTHETIC_STREAMS:
        got = wpt.lzss_decompress(stream, org)
        assert got == expect, f'合成流自证失败: {desc}'
        idx += 1
        write_bytes(GOLDEN / 'decomp' / f'{idx:03d}.bin', stream)
        write_bytes(GOLDEN / 'decomp' / f'{idx:03d}.out', expect)
        small_json.append({'id': idx, 'pak': '(synthetic)', 'inner': desc,
                           'org': org, 'synthetic': True,
                           'stored_file': f'decomp/{idx:03d}.bin',
                           'out_file': f'decomp/{idx:03d}.out',
                           'stored_sha256': sha256_bytes(stream),
                           'out_sha256': sha256_bytes(expect)})

    write_json(GOLDEN / 'lzss_decompress_cases.json',
               {'small': small_json, 'large': cases_large})
    log(f'解压向量: small={len(small_json)}（含合成 {len(SYNTHETIC_STREAMS)}）, '
        f'large={len(cases_large)}')
    return {'small': len(small_json), 'large': len(cases_large)}


# --------------------------------------------------------------------------
# 4. LZSS 压缩合成边界向量
# --------------------------------------------------------------------------
def _case_input_list():
    """(desc, bytes) 合成输入清单。"""
    txt = (ETC_DIR / 'server_res_text.txt').read_bytes()
    return [
        ('empty', b''),
        ('literal-1byte', b'A'),
        ('literal-2byte', b'AB'),
        ('literal-3byte-minmatch-threshold', b'ABC'),
        ('repeat-17', b'Z' * 17),
        ('repeat-18-maxmatch', b'Z' * 18),
        ('repeat-19', b'Z' * 19),
        ('repeat-20', b'Z' * 20),
        ('same-4077', b'Q' * 4077),
        ('same-4078-window-boundary', b'Q' * 4078),
        ('same-4079', b'Q' * 4079),
        ('same-4096', b'Q' * 4096),
        ('spaces-512-initial-window', b' ' * 512),
        ('cross-abc-x1000', b'abc' * 1000),
        ('random-64B', hashlib.shake_256(b'golden-rnd-64B').digest(64)),
        ('random-4KB', hashlib.shake_256(b'golden-rnd-4KB').digest(4096)),
        ('random-64KB', hashlib.shake_256(b'golden-rnd-64KB').digest(65536)),
        ('random-256KB', hashlib.shake_256(b'golden-rnd-256KB').digest(262144)),
        ('text-head-1KB', txt[:1024]),
        ('text-tail-1KB', txt[-1024:]),
    ]


def gen_compress_cases():
    cases = []
    for cid, (desc, data) in enumerate(_case_input_list(), 1):
        t0 = time.time()
        blob = wpt.lzss_compress(data)
        # 自校验：往返必须一致（全部通过才写入）
        back = wpt.lzss_decompress(blob, len(data))
        assert back == data, f'压缩向量自校验失败: {desc}'
        entry = {'id': cid, 'desc': desc, 'input_len': len(data),
                 'input_sha256': sha256_bytes(data),
                 'expect_output_len': len(blob),
                 'expect_output_sha256': sha256_bytes(blob)}
        if len(data) <= HEX_INLINE_MAX:
            entry['input_hex'] = data.hex()
        else:
            write_bytes(GOLDEN / 'comp' / f'case_{cid:02d}.in', data)
            entry['input_file'] = f'comp/case_{cid:02d}.in'
        if len(blob) <= HEX_INLINE_MAX:
            entry['expect_output_hex'] = blob.hex()
        else:
            write_bytes(GOLDEN / 'comp' / f'case_{cid:02d}.out', blob)
            entry['expect_output_file'] = f'comp/case_{cid:02d}.out'
        cases.append(entry)
        log(f'压缩向量 {cid:02d} {desc}: {len(data)}B -> {len(blob)}B '
            f'({time.time() - t0:.1f}s)')
    write_json(GOLDEN / 'lzss_compress_cases.json', {'cases': cases})
    return {'count': len(cases)}


# --------------------------------------------------------------------------
# 5. LZSS 压缩抽样向量（etc.pak 解包目录，40 个）
# --------------------------------------------------------------------------
LARGE_SAMPLE = ['gbsn00lp.ttf', 'FZKTJW.ttf', 'server_res_text.txt']  # >256KB 只记哈希
SPECIAL_SAMPLE = ['task_score.list',        # 0 字节真实空文件
                  'zuanshi -test.list',     # 特殊 ASCII 名
                  'Font1.ttf', 'Font2.ttf', 'Font4.ttf', 'Font5.ttf',
                  'boss_template.list',     # 261917B，近 256KB 边界
                  'ac_combat_answer.c']
BUCKET_QUOTA = [(0, 1024, 5), (1024, 8192, 8),
                (8192, 65536, 10), (65536, 256 * 1024, 6)]  # (lo, hi, 取前 N)


def gen_compress_files():
    entries = []
    chosen = set(LARGE_SAMPLE) | set(SPECIAL_SAMPLE)
    all_files = []
    for n in os.listdir(ETC_DIR):
        p = ETC_DIR / n
        if n != wpt.MANIFEST_NAME and p.is_file():
            all_files.append((p.stat().st_size, n))
    for lo, hi, q in BUCKET_QUOTA:
        cands = sorted((s, n) for s, n in all_files
                       if lo <= s < hi and n not in chosen)[:q]
        chosen.update(n for _, n in cands)
    final = sorted(chosen)
    assert len(final) == 40, f'抽样数量 {len(final)} != 40'

    for cid, name in enumerate(final, 1):
        data = (ETC_DIR / name).read_bytes()
        t0 = time.time()
        blob = wpt.lzss_compress(data)
        assert wpt.lzss_decompress(blob, len(data)) == data, f'往返失败: {name}'
        entry = {'id': cid, 'path': name, 'input_len': len(data),
                 'input_sha256': sha256_bytes(data),
                 'expect_output_len': len(blob),
                 'expect_output_sha256': sha256_bytes(blob),
                 'input_source': 'etc.pak/' + name}
        if len(data) <= COMP_FILE_LIMIT:
            write_bytes(GOLDEN / 'comp' / f'file_{cid:02d}.in', data)
            write_bytes(GOLDEN / 'comp' / f'file_{cid:02d}.out', blob)
            entry['in_file'] = f'comp/file_{cid:02d}.in'
            entry['out_file'] = f'comp/file_{cid:02d}.out'
        entries.append(entry)
        log(f'抽样压缩 {cid:02d}/40 {name}: {len(data)}B -> {len(blob)}B '
            f'({time.time() - t0:.1f}s)')
    write_json(GOLDEN / 'lzss_compress_files.json', {'files': entries})
    return {'count': len(entries),
            'stored_inline': sum(1 for e in entries if 'in_file' in e)}


# --------------------------------------------------------------------------
# 6. replace 向量
# --------------------------------------------------------------------------
REPLACE_NEWFILE = b'REPLACED-' * 500   # 5000B 固定内容
REPLACE_TARGETS = [
    ('etc.pak', '100heis.list', ('compress', 'no-compress')),
    ('lib_gs32.pak', 'clone/misc/mixed_agent.o', ('compress',)),
]


def verify_replace(new_pak, orig_pak, inner, expect):
    np_ = wpt.Pak(str(new_pak))
    op_ = wpt.Pak(str(orig_pak))
    try:
        node = np_.find(inner)
        assert node is not None and np_.read_file(node) == expect, f'{inner} 替换内容不符'
        checked = 0
        for rel, e in walk_files(op_):        # 抽前 3 个其他文件比对（字节原样保留）
            if rel == inner or checked >= 3:
                continue
            assert np_.read_file(np_.find(rel)) == op_.read_file(e), f'{rel} 未原样保留'
            checked += 1
    finally:
        np_.close()
        op_.close()


def gen_replace():
    write_bytes(GOLDEN / 'replace' / 'newfile.bin', REPLACE_NEWFILE)
    newfile_path = GOLDEN / 'replace' / 'newfile.bin'
    cases = []
    for pakname, inner, modes in REPLACE_TARGETS:
        before = sha256_file(PAKS[pakname])
        for mode in modes:
            with tmp_dir('replace') as d:
                tmp_pak = d / pakname
                shutil.copyfile(str(PAKS[pakname]), str(tmp_pak))
                assert sha256_file(tmp_pak) == before
                args = ['replace', tmp_pak, inner, newfile_path]
                if mode == 'no-compress':
                    args.append('--no-compress')
                run_cli(args)
                verify_replace(tmp_pak, PAKS[pakname], inner, REPLACE_NEWFILE)
                after = sha256_file(tmp_pak)
                cases.append({'pak': pakname, 'inner': inner,
                              'newfile': 'replace/newfile.bin', 'mode': mode,
                              'pak_sha256_before': before,
                              'pak_sha256_after': after})
                log(f'replace {pakname}:{inner} [{mode}] -> {after[:16]}…')
    write_json(GOLDEN / 'replace' / 'replace.json',
               {'newfile_len': len(REPLACE_NEWFILE), 'cases': cases})
    return {'cases': len(cases)}


# --------------------------------------------------------------------------
# 7. 总索引
# --------------------------------------------------------------------------
def golden_stats():
    n, total = 0, 0
    for p in GOLDEN.rglob('*'):
        if p.is_file():
            n += 1
            total += p.stat().st_size
    return n, total


def main():
    t0 = time.time()
    for sub in ('decomp', 'comp'):
        rmtree_owned(GOLDEN / sub, '重建向量子目录')
    GOLDEN.mkdir(parents=True, exist_ok=True)

    layout = gen_layout()
    extract = gen_extract()
    decomp = gen_decompress(GOLDEN / 'layout' / 'etc_mini_nocompress.pak')
    ccases = gen_compress_cases()
    cfiles = gen_compress_files()
    replace = gen_replace()

    total_files, total_bytes = golden_stats()
    write_json(GOLDEN / 'golden_index.json', {
        'version': 1,
        'generated_at': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
        'python_tool_version': wpt.__version__,
        'vectors': {
            'extract': {'manifest': 'extract_files.sha256', **extract},
            'decompress': {'index': 'lzss_decompress_cases.json',
                           'data_dir': 'decomp', **decomp},
            'compress_cases': {'index': 'lzss_compress_cases.json',
                               'data_dir': 'comp', **ccases},
            'compress_files': {'index': 'lzss_compress_files.json',
                               'data_dir': 'comp', **cfiles},
            'layout': {'index': 'layout/layout.json', **layout},
            'replace': {'index': 'replace/replace.json', **replace},
        },
        'total_files': total_files,
        'total_bytes': total_bytes,
        'notes': NOTES,
    })
    dt = time.time() - t0
    print('\n===== 生成摘要 =====')
    print(f'extract 清单行数     : {extract["lines"]} '
          f'({", ".join(k + "=" + str(v["files"]) for k, v in extract["paks"].items())})')
    print(f'解压向量             : small={decomp["small"]} large={decomp["large"]}')
    print(f'压缩边界向量         : {ccases["count"]}')
    print(f'压缩抽样向量         : {cfiles["count"]}（落盘 {cfiles["stored_inline"]}）')
    print(f'布局向量             : 输入 {layout["input_files"]} 文件 × 2 pak')
    print(f'replace 向量         : {replace["cases"]} 条')
    print(f'golden/ 总计         : {total_files} 文件, {total_bytes / 1e6:.1f} MB')
    print(f'总耗时               : {dt:.0f}s')


if __name__ == '__main__':
    main()