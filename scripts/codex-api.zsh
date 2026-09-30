#!/bin/zsh
set -euo pipefail

routing_config="${1:?缺少 router 配置路径}"
catalog_directory="${2:?缺少模型目录保存位置}"
client_program="${3:?缺少 Codex 程序}"
command_name="${ROUTER_COMMAND_NAME:?缺少启动命令标识，请通过 router run 启动}"
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
catalog_source="$(mktemp "$catalog_directory/source.XXXXXXXX")"
catalog_file="$(mktemp "$catalog_directory/catalog.XXXXXXXX")"
trap 'rm -f -- "$catalog_source" "$catalog_file"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

"$client_program" "${provider_args[@]}" debug models > "$catalog_source"

# 根据命令绑定筛选目录，并保留上游能力与服务等级信息。
jq --slurpfile routing "$routing_config" --arg command "$command_name" \
  --argjson defaults "${ROUTER_CODEX_DEFAULT_SERVICE_TIERS:-[]}" \
  --arg pattern "${ROUTER_CODEX_CATALOG_MODEL_PATTERN:-}" \
  -f "${0:A:h}/codex-catalog.jq" "$catalog_source" > "$catalog_file"
rm -- "$catalog_source"

catalog_option="$(jq -nr --arg path "$catalog_file" '"model_catalog_json=" + ($path | tojson)')"
"$client_program" "${provider_args[@]}" -c "$catalog_option" "$@"
