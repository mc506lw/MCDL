#!/usr/bin/env bash
# 版本隔离真跑：隔离开 / 隔离关 各装一次，看模组落在哪。
set -o pipefail
cd "$(dirname "$0")/.." || exit 1
BIN=./target-iso/debug/mcdl.exe
rm -rf tmp-iso/iso tmp-iso/shared
mkdir -p tmp-iso

echo "===== 1) 隔离开（默认） ====="
$BIN --dir tmp-iso/iso install 1.21.1 --loader fabric --fabric-api --no-assets 2>&1 | tail -30
echo
echo "===== 2) 隔离关（--no-isolate） ====="
$BIN --dir tmp-iso/shared install 1.21.1 --loader fabric --fabric-api --no-assets --no-isolate 2>&1 | tail -30
echo
echo "===== 磁盘结果 ====="
echo "--- tmp-iso/iso 里的 mods 目录："
find tmp-iso/iso -maxdepth 4 -type d -name mods | sort
echo "--- tmp-iso/iso 里的 jar（模组）："
find tmp-iso/iso -path '*mods*' -name '*.jar' | sort
echo "--- tmp-iso/shared 里的 mods 目录："
find tmp-iso/shared -maxdepth 4 -type d -name mods | sort
echo "--- tmp-iso/shared 里的 jar（模组）："
find tmp-iso/shared -path '*mods*' -name '*.jar' | sort
