#!/bin/zsh
set -euo pipefail

routing_config="${1:?缺少 router 配置路径}"
catalog_directory="${2:?缺少模型目录保存位置}"
client_program="${3:?缺少 Codex 程序}"
router_program="${4:?缺少 Router 程序}"
command_name="${ROUTER_COMMAND_NAME:?缺少启动命令标识，请通过 router run 启动}"
shift 4

typeset -a provider_args=()
while (( $# )) && [[ "$1" != "--" ]]; do
  provider_args+=("$1")
  shift
done
[[ "${1:-}" == "--" ]] || { echo "缺少启动参数分隔符" >&2; exit 1; }
shift

mkdir -p "$catalog_directory"
chmod 700 "$catalog_directory"
client_umask="$(umask)"
umask 077
catalog_native=""
catalog_file="$(mktemp "$catalog_directory/catalog.XXXXXXXX")"
trap 'rm -f -- "$catalog_file" "$catalog_native"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
catalog_native="$(mktemp "$catalog_directory/native.XXXXXXXX")"
(umask "$client_umask"; "$client_program" --no-daemon debug models --bundled) > "$catalog_native"
"${ROUTER_CODEX_CATALOG_PYTHON:-python3}" "${0:A:h}/codex-model-source.py" "$routing_config" "$command_name" "$catalog_native" > "$catalog_file"

catalog_option="$(jq -nr --arg path "$catalog_file" '"model_catalog_json=" + ($path | tojson)')"
umask "$client_umask"
"$router_program" __client-exec "$client_program" "${provider_args[@]}" -c "$catalog_option" "$@"
