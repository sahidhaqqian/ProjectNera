#!/usr/bin/env python3
"""
Simple code stats scanner for Git repos
Scan folder dan hitung total baris kode yang kamu tulis
Support single repo atau multiple repo dalam satu folder
"""

import os
import subprocess
import argparse
from pathlib import Path
from datetime import datetime

# Ekstensi yang mau di-scan
EXTENSIONS = {
    '.rs', '.py', '.js', '.ts', '.jsx', '.tsx',
    '.go', '.java', '.cpp', '.c', '.h', '.hpp',
    '.rb', '.php', '.swift', '.kt', '.scala',
    '.html', '.css', '.scss', '.sass', '.vue',
    '.sql', '.sh', '.bash', '.zsh'
}

# Ekstensi yang di-ignore
IGNORE_EXT = {
    '.lock', '.log', '.tmp', '.json', '.xml', '.yaml', '.yml',
    '.toml', '.ini', '.cfg', '.conf', '.env', '.gitignore',
    '.png', '.jpg', '.jpeg', '.gif', '.svg', '.ico',
    '.mp4', '.mp3', '.wav', '.pdf', '.doc', '.zip', '.tar'
}

IGNORE_DIRS = {
    'node_modules', 'target', 'dist', 'build',
    '__pycache__', 'venv', 'env', '.idea', '.vscode',
    'coverage', 'logs', '.cache'
}


def get_git_stats(repo_path):
    """Get Git commit stats untuk repo"""
    original_dir = os.getcwd()
    os.chdir(repo_path)

    try:
        # Get author email
        email = subprocess.check_output(
            ['git', 'config', 'user.email'],
            text=True
        ).strip()

        # Get total commits
        commit_count = subprocess.check_output(
            ['git', 'rev-list', '--count', 'HEAD', '--author=' + email],
            text=True
        ).strip()

        # Get added/deleted lines
        stats = subprocess.check_output(
            ['git', 'log', '--author=' + email, '--pretty=tformat:', '--numstat'],
            text=True
        )

        added = 0
        deleted = 0
        for line in stats.split('\n'):
            if line.strip():
                parts = line.split()
                if len(parts) >= 2 and parts[0].isdigit() and parts[1].isdigit():
                    added += int(parts[0])
                    deleted += int(parts[1])

        os.chdir(original_dir)
        return {
            'commits': int(commit_count),
            'added': added,
            'deleted': deleted,
            'net': added - deleted,
            'email': email
        }

    except (subprocess.CalledProcessError, FileNotFoundError):
        os.chdir(original_dir)
        return None


def scan_files(repo_path):
    """Scan semua file di folder"""
    total_lines = 0
    file_count = 0
    files_by_ext = {}

    for root, dirs, files in os.walk(repo_path):
        # Skip ignored directories
        dirs[:] = [d for d in dirs if d not in IGNORE_DIRS]

        # Skip .git
        if '.git' in root:
            continue

        for file in files:
            file_path = os.path.join(root, file)
            ext = os.path.splitext(file)[1].lower()

            # Skip ignored extensions
            if ext in IGNORE_EXT:
                continue

            # Count only extensions kita atau file tanpa ekstensi
            if ext and ext not in EXTENSIONS:
                continue

            try:
                with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
                    lines = len(f.readlines())
                    total_lines += lines
                    file_count += 1

                    if ext not in files_by_ext:
                        files_by_ext[ext] = 0
                    files_by_ext[ext] += lines
            except (UnicodeDecodeError, PermissionError, IsADirectoryError):
                continue

    return total_lines, file_count, files_by_ext


def scan_repos(root_path):
    """Scan repo di root_path"""
    results = []
    root = Path(root_path)

    # CEK: Apakah root itu sendiri adalah repo?
    if (root / '.git').exists() and (root / '.git').is_dir():
        print(f"📁 Scanning: {root.name}")
        git_stats = get_git_stats(str(root))
        total_lines, file_count, files_by_ext = scan_files(str(root))
        results.append({
            'name': root.name,
            'path': str(root),
            'git': git_stats,
            'lines': total_lines,
            'files': file_count,
            'by_ext': files_by_ext
        })
        return results  # Langsung return

    # KALO BUKAN: Scan subfolder
    for item in root.iterdir():
        if item.is_dir():
            git_path = item / '.git'
            if git_path.exists() and git_path.is_dir():
                print(f"📁 Scanning: {item.name}")
                git_stats = get_git_stats(str(item))
                total_lines, file_count, files_by_ext = scan_files(str(item))
                results.append({
                    'name': item.name,
                    'path': str(item),
                    'git': git_stats,
                    'lines': total_lines,
                    'files': file_count,
                    'by_ext': files_by_ext
                })

    return results


def print_results(results):
    """Print hasil scan"""
    print("\n" + "=" * 70)
    print("📊 CODE STATISTICS REPORT")
    print("=" * 70)

    total_lines = 0
    total_files = 0
    total_commits = 0
    total_added = 0
    total_deleted = 0

    for repo in results:
        print(f"\n📦 {repo['name']}")
        print(f"   📁 {repo['files']:,} files")
        print(f"   📝 {repo['lines']:,} lines")

        if repo['git']:
            print(f"   📌 {repo['git']['commits']} commits (by {repo['git']['email']})")
            print(f"   ➕ {repo['git']['added']:,} lines added")
            print(f"   ➖ {repo['git']['deleted']:,} lines deleted")
            print(f"   📈 Net: {repo['git']['net']:,}")

            total_commits += repo['git']['commits']
            total_added += repo['git']['added']
            total_deleted += repo['git']['deleted']

        if repo['by_ext']:
            sorted_ext = sorted(repo['by_ext'].items(), key=lambda x: x[1], reverse=True)[:3]
            ext_str = ', '.join([f"{ext}: {count:,}" for ext, count in sorted_ext])
            print(f"   🏷️  {ext_str}")

        total_lines += repo['lines']
        total_files += repo['files']

    print("\n" + "=" * 70)
    print("📈 SUMMARY")
    print("=" * 70)
    print(f"📦 Total repos: {len(results)}")
    print(f"📁 Total files: {total_files:,}")
    print(f"📝 Total lines: {total_lines:,}")

    if total_commits > 0:
        print(f"📌 Total commits: {total_commits:,}")
        print(f"➕ Total lines added: {total_added:,}")
        print(f"➖ Total lines deleted: {total_deleted:,}")
        print(f"📈 Net lines: {total_added - total_deleted:,}")

    print("\n" + "=" * 70)
    print(f"🕐 Scanned at: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 70)


def main():
    parser = argparse.ArgumentParser(
        description='Scan Git repos dan hitung baris kode',
        epilog='Contoh: python code_stats.py ~/projects'
    )
    parser.add_argument(
        'path', nargs='?', default='.',
        help='Path untuk di-scan (default: current directory)'
    )
    parser.add_argument(
        '--ext', '-e',
        help='Tambahan ekstensi (contoh: .zig,.v)'
    )
    parser.add_argument(
        '--ignore', '-i',
        help='Folder tambahan untuk di-ignore (contoh: build,dist)'
    )
    parser.add_argument(
        '--no-git', action='store_true',
        help='Scan semua folder (tidak hanya repo Git)'
    )
    parser.add_argument(
        '--single', action='store_true',
        help='Scan satu folder tanpa cek .git'
    )

    args = parser.parse_args()

    # Tambah custom extensions
    if args.ext:
        for ext in args.ext.split(','):
            ext = ext.strip()
            if not ext.startswith('.'):
                ext = '.' + ext
            EXTENSIONS.add(ext)

    # Tambah custom ignore dirs
    if args.ignore:
        for d in args.ignore.split(','):
            IGNORE_DIRS.add(d.strip())

    # Mode single folder (no git required)
    if args.single:
        print(f"🔍 Scanning: {os.path.abspath(args.path)}")
        total_lines, file_count, files_by_ext = scan_files(args.path)
        results = [{
            'name': Path(args.path).name,
            'path': str(Path(args.path).resolve()),
            'git': None,
            'lines': total_lines,
            'files': file_count,
            'by_ext': files_by_ext
        }]
        print_results(results)
        return

    # Mode no-git: scan semua folder
    if args.no_git:
        print(f"🔍 Scanning: {os.path.abspath(args.path)}")
        print("📂 Mode: No-Git (scan semua folder)\n")
        results = []
        root = Path(args.path)

        for item in root.iterdir():
            if item.is_dir():
                print(f"📁 Scanning: {item.name}")
                total_lines, file_count, files_by_ext = scan_files(str(item))
                results.append({
                    'name': item.name,
                    'path': str(item),
                    'git': None,
                    'lines': total_lines,
                    'files': file_count,
                    'by_ext': files_by_ext
                })

        if not results:
            print("❌ No folders found!")
            return
        print_results(results)
        return

    # Mode normal: scan Git repos
    print(f"🔍 Scanning: {os.path.abspath(args.path)}")
    print("📂 Looking for Git repositories...\n")

    results = scan_repos(args.path)

    if not results:
        print("❌ No Git repositories found!")
        print("💡 Tips:")
        print("   - Gunakan --single untuk scan satu folder")
        print("   - Gunakan --no-git untuk scan semua folder")
        print("   - Pastikan path benar dan ada .git di dalamnya")
        return

    print_results(results)


if __name__ == "__main__":
    main()