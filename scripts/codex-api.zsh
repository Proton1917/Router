#!/bin/zsh
set -euo pipefail

routing_config="${1:?缺少 router 配置路径}"
catalog_directory="${2:?缺少模型目录保存位置}"
client_program="${3:?缺少 Codex 程序}"
command_name="${ROUTER_COMMAND_NAME:?缺少启动命令标识，请通过 router run 启动}"
cache_seconds="${ROUTER_CODEX_CATALOG_CACHE_SECONDS:-300}"
[[ "$cache_seconds" =~ '^(0|[1-9][0-9]*)$' ]] || { echo "模型目录缓存秒数必须为非负整数" >&2; exit 1; }
shift 3

typeset -a provider_args=()
while (( $# )) && [[ "$1" != "--" ]]; do
  provider_args+=("$1")
  shift
done
[[ "${1:-}" == "--" ]] || { echo "缺少启动参数分隔符" >&2; exit 1; }
shift

mkdir -p "$catalog_directory"
chmod 700 "$catalog_directory"
umask 077
catalog_source="$(mktemp "$catalog_directory/source.XXXXXXXX")"
catalog_file=""
trap 'rm -f -- "$catalog_source" "$catalog_file"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
catalog_file="$(mktemp "$catalog_directory/catalog.XXXXXXXX")"

client_version="$("$client_program" --version)"
cache_id="$(printf '%s\0' "${routing_config:A}" "$command_name" | shasum -a 256)"
cache_id="${cache_id%% *}"
catalog_cache="$catalog_directory/cache.$cache_id.json"
fingerprint="$(
  { printf '%s\0' "$client_program" "$client_version" "${provider_args[@]}" \
      "${ROUTER_CODEX_DEFAULT_SERVICE_TIERS:-[]}" "${ROUTER_CODEX_CATALOG_MODEL_PATTERN:-}";
    cat -- "$routing_config" "${0:A}" "${0:A:h}/codex-catalog.jq"; } | shasum -a 256
)"
fingerprint="${fingerprint%% *}"
zmodload zsh/system
: >> "$catalog_directory/cache.$cache_id.lock"
zsystem flock -t 30 -f cache_lock "$catalog_directory/cache.$cache_id.lock" || {
  echo "等待模型目录更新锁超时" >&2
  exit 1
}
cache_valid=false
if [[ -f "$catalog_cache" ]] && (( cache_seconds > 0 )); then
  cache_valid="$(jq -er --arg fingerprint "$fingerprint" --argjson now "$(date +%s)" \
    --argjson ttl "$cache_seconds" '
      if (.fingerprint | type) != "string" or (.created_at | type) != "number"
         or (.catalog.models | type) != "array" or (.catalog.models | length) == 0
      then error("已保存的模型目录损坏，请移除该缓存文件后重新启动")
      else (.fingerprint == $fingerprint and .created_at <= $now and $now - .created_at < $ttl) | tostring end
    ' "$catalog_cache")"
fi

if [[ "$cache_valid" == true ]]; then
  jq -e '.catalog' "$catalog_cache" > "$catalog_file"
else
  "$client_program" "${provider_args[@]}" debug models > "$catalog_source"
  # 校验命令绑定与真实目录元数据后，原子保存精简目录。
  if ! jq --slurpfile routing "$routing_config" --arg command "$command_name" \
    --argjson defaults "${ROUTER_CODEX_DEFAULT_SERVICE_TIERS:-[]}" \
    --arg pattern "${ROUTER_CODEX_CATALOG_MODEL_PATTERN:-}" \
    -f "${0:A:h}/codex-catalog.jq" "$catalog_source" > "$catalog_file"; then
    echo "router: $command_name 的模型目录校验失败，启动已停止；请检查后端目录连接与模型绑定。" >&2
    exit 1
  fi
  if (( cache_seconds > 0 )); then
    jq -c --arg fingerprint "$fingerprint" --argjson created_at "$(date +%s)" \
      '{fingerprint:$fingerprint, created_at:$created_at, catalog:.}' "$catalog_file" > "$catalog_source"
    mv -- "$catalog_source" "$catalog_cache"
  fi
fi
zsystem flock -u "$cache_lock"
[[ ! -f "$catalog_source" ]] || rm -- "$catalog_source"

catalog_option="$(jq -nr --arg path "$catalog_file" '"model_catalog_json=" + ($path | tojson)')"
"$client_program" "${provider_args[@]}" -c "$catalog_option" "$@"
