#!/bin/zsh
set -euo pipefail

routing_config="${1:?缺少 router 配置路径}"
catalog_directory="${2:?缺少模型目录保存位置}"
client_program="${3:?缺少 Codex 程序}"
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
trap 'rm -- "$catalog_source" "$catalog_file"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

"$client_program" "${provider_args[@]}" debug models > "$catalog_source"

# 合并启动方式的默认服务等级、后端目录元数据和单个模型的配置。
jq --slurpfile routing "$routing_config" --argjson defaults "${ROUTER_CODEX_DEFAULT_SERVICE_TIERS:-[]}" --arg pattern "${ROUTER_CODEX_CATALOG_MODEL_PATTERN:-}" '
  if ($defaults | type) != "array" or any($defaults[]; (.id | type) != "string" or (.name | type) != "string")
  then error("默认服务等级必须为包含 id 和 name 的对象数组") else . end
  | if $pattern != "" and (any(.models[]; .slug | test($pattern)) | not)
    then error("后端模型目录没有符合入口配置的模型") else . end
  |
  $routing[0] as $config
  | .models |= map(
      . as $model
      | [$config.profiles[]
         | select(.standard.model == $model.slug and .fast.model == $model.slug and .standard.target == .fast.target)
         | .fast.field_policy.set["/service_tier"] // empty] | unique as $tiers
      | $model
      | .service_tiers = ($defaults + (.service_tiers // []) + [$tiers[] as $tier | {
          id: $tier,
          name: ($config.management.labels["service-tier:" + $tier] // ([$defaults[] | select(.id == $tier) | .name][0]) // $tier),
          description: ($config.management.labels["service-tier-description:" + $tier] // ([$defaults[] | select(.id == $tier) | .description][0]) // $tier)
        }] | group_by(.id) | map(last))
    )
' "$catalog_source" > "$catalog_file"

catalog_option="$(jq -nr --arg path "$catalog_file" '"model_catalog_json=" + ($path | tojson)')"
"$client_program" "${provider_args[@]}" -c "$catalog_option" "$@"
