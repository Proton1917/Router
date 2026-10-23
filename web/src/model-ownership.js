const commands = config => Object.entries(config.management.commands || {});
const integrations = config => Object.entries(config.management.integrations || {});

function routeOwners(config) {
  const owners = new Map();
  for (const [command, spec] of commands(config)) {
    for (const binding of spec.model_bindings || []) {
      for (const id of binding.route_ids || []) owners.set(id, {command, binding:binding.id, profile:binding.profile});
    }
  }
  return owners;
}

export function profileUsages(config, profile) {
  const usages = [];
  const owned = routeOwners(config);
  const generated = new Set(config.management.generated_routes || []);
  for (const [command, spec] of commands(config)) {
    if (spec.profile === profile) usages.push({kind:'command', command});
    for (const binding of spec.model_bindings || []) {
      if (binding.profile === profile) usages.push({kind:'binding', command, binding:binding.id});
    }
  }
  for (const [integration, spec] of integrations(config)) {
    for (const slot of spec.slots || []) {
      if (slot.profile === profile) usages.push({kind:'integration', integration, slot:slot.id});
    }
  }
  for (const route of config.routes) {
    if (!generated.has(route.id) && !owned.has(route.id) && route.profile === profile) usages.push({kind:'route', route:route.id});
  }
  return usages;
}

export function isExclusiveCommandProfile(config, profile, command, binding) {
  return profileUsages(config, profile).every(usage => usage.kind === 'binding' && usage.command === command && usage.binding === binding);
}

export function customizeCommandProfile(config, command, binding, modify) {
  const item = config.management.commands[command].model_bindings.find(item => item.id === binding);
  const id = isExclusiveCommandProfile(config,item.profile,command,binding) ? item.profile : `command-model-${crypto.randomUUID().replaceAll('-','')}`;
  const profile = structuredClone(config.profiles[item.profile]);
  modify(profile);
  return {id,profile};
}

export function pruneDetachedCommandProfiles(config, previous) {
  const candidates = new Set([
    previous?.profile,
    ...(previous?.model_bindings || []).map(binding => binding.profile),
    ...Object.keys(config.profiles).filter(id => id.startsWith('command-model-')),
  ]);
  for (const id of candidates) {
    if (id && config.profiles[id] && !profileUsages(config, id).length) {
      delete config.profiles[id];
      delete config.management.profile_models?.[id];
      delete config.management.labels?.[`profile:${id}`];
    }
  }
}

export function plannedProfileRouteCount(config, profile) {
  const generated = new Set(config.management.generated_routes || []);
  const owners = routeOwners(config);
  let count = config.routes.filter(route => !generated.has(route.id) && (owners.get(route.id)?.profile || route.profile) === profile).length;
  for (const [, spec] of commands(config)) {
    if (spec.profile === profile) count++;
    count += (spec.model_bindings || []).filter(binding => binding.profile === profile).length;
    count += (spec.model_bindings || []).filter(binding => binding.compact_with_self && binding.profile === profile).length * (config.management.templates[spec.template]?.compaction_routes?.length || 0);
    count += (spec.model_bindings || []).filter(binding => binding.subagents_with_self && binding.profile === profile).length * (config.management.templates[spec.template]?.subagent_routes?.length || 0);
  }
  for (const [, spec] of integrations(config)) count += (spec.slots || []).filter(slot => slot.profile === profile).length;
  return count;
}
