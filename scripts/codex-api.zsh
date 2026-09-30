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

# 从模型配置补充客户端服务等级，保留后端目录中的其他模型元数据。
jq --slurpfile routing "$routing_config" '
  $routing[0] as $config
  | .models |= map(
      . as $model
      | [$config.profiles[]
         | select(.standard.model == $model.slug and .fast.model == $model.slug and .standard.target == .fast.target)
         | .fast.field_policy.set["/service_tier"] // empty] | unique as $tiers
      | $model
      | .service_tiers = ((.service_tiers // []) + [$tiers[] as $tier | {
          id: $tier,
          name: ($config.management.labels["service-tier:" + $tier] // $tier),
          description: ($config.management.labels["service-tier-description:" + $tier] // $tier)
        }] | unique_by(.id))
    )
' "$catalog_source" > "$catalog_file"

catalog_option="$(jq -nr --arg path "$catalog_file" '"model_catalog_json=" + ($path | tojson)')"
"$client_program" "${provider_args[@]}" -c "$catalog_option" "$@"
