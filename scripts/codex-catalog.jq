$routing[0] as $config
| $config.management.commands[$command] as $entry
| if $entry == null then error("当前启动命令没有对应的 router 配置") else . end
| if ($defaults | type) != "array" or any($defaults[]; (.id | type) != "string" or (.name | type) != "string")
  then error("默认服务等级必须为包含 id 和 name 的对象数组") else . end
| if (.models | type) != "array" then error("Codex 返回了无效的模型目录") else . end
| if $pattern != "" and (any(.models[]; .slug | test($pattern)) | not)
  then error("Codex 未返回当前后端的模型目录；远程获取失败时可能返回内置目录，即使命令退出状态为成功") else . end
| ([ $entry.profile // empty ] + [
    $config.routes[]
    | select(any(.match.headers[]?;
        (.name | ascii_downcase) == ($config.management.command_header | ascii_downcase)
        and any(.values[]?; . == $command)))
    | .profile
  ] | unique) as $profiles
| ([$profiles[] as $id
    | $config.profiles[$id] as $profile
    | ($profile.standard.model //
        (if $id == $entry.profile then $entry.model else null end)) as $upstream
    | if ($upstream | type) != "string" or $upstream == ""
      then error("命令绑定的模型配置需要明确的 API 模型 ID：" + $id)
      else {
        profile: $profile,
        upstream: $upstream,
        client: (if $id == $entry.profile and ($entry.model // "") != ""
          then $entry.model
          else ($config.management.profile_models[$id].responses // $upstream) end)
      } end
  ] + (if ($profiles | length) == 0 and ($entry.model // "") != ""
       then [{profile: null, upstream: $entry.model, client: $entry.model}] else [] end)) as $bindings
| if ($bindings | length) == 0 then error("当前命令尚未配置可选模型，请先设置模型或专属模型路由") else . end
| .models as $models
| .models = [$bindings[] as $binding
    | ([$models[] | select(.slug == $binding.upstream)] | first) as $model
    | if $model == null
      then error("Codex 目录缺少已配置模型的元数据：" + $binding.upstream)
      else $model end
    | .slug = $binding.client
    | .service_tiers = ($defaults + ($model.service_tiers // []) + [
        $binding.profile
        | select(.standard.model == $binding.upstream and .fast.model == $binding.upstream and .standard.target == .fast.target)
        | .fast.field_policy.set["/service_tier"] // empty
        | . as $tier
        | {
            id: $tier,
            name: ($config.management.labels["service-tier:" + $tier] // ([$defaults[] | select(.id == $tier) | .name][0]) // $tier),
            description: ($config.management.labels["service-tier-description:" + $tier] // ([$defaults[] | select(.id == $tier) | .description][0]) // $tier)
          }
      ] | group_by(.id) | map(last))
  ]
| if ([.models[].slug] | length) != ([.models[].slug] | unique | length)
  then error("当前命令的模型配置使用了重复的客户端模型 ID") else . end
