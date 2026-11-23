function effortSuffixes(variants) {
  return Object.entries(variants?.suffixes || {}).filter(([, suffix]) => suffix).sort((a, b) => b[1].length - a[1].length);
}

export function modelFamilyId(model, settings) {
  const match = effortSuffixes(settings?.reasoning_variants).find(([, suffix]) => model.endsWith(suffix));
  return match ? model.slice(0, -match[1].length) : model;
}

export function catalogModelMatches(row, id) {
  return row.id === id || row.variantIds?.includes(id);
}

export function groupReasoningModels(models, settings) {
  const variants = settings?.reasoning_variants;
  if (!variants) return models;
  const suffixes = effortSuffixes(variants);
  const families = new Map();
  for (const row of models) {
    const match = suffixes.find(([, suffix]) => row.id.endsWith(suffix));
    if (!match) continue;
    const [effort, suffix] = match;
    const id = row.id.slice(0, -suffix.length);
    const nameSuffix = variants.name_suffixes[effort];
    const name = nameSuffix && row.name.endsWith(nameSuffix) ? row.name.slice(0, -nameSuffix.length) : row.name;
    if (!families.has(id)) families.set(id, []);
    families.get(id).push({ row, effort, name: row.name === row.id ? id : name });
  }

  const groups = new Map();
  for (const [id, entries] of [...families].sort(([a], [b]) => a.length - b.length)) {
    const selected = entries.find(entry => entry.effort === variants.default_effort) || entries[0];
    const { name } = selected;
    const parent = groups.get(name)?.find(group => id.startsWith(`${group.id}-`));
    if (parent) {
      parent.entries.push(...entries);
    } else {
      if (!groups.has(name)) groups.set(name, []);
      groups.get(name).push({ id, selected, entries });
    }
  }

  const grouped = new Map();
  for (const [name, namedGroups] of groups) {
    for (const { id, selected, entries } of namedGroups) {
      const targets = {};
      for (const { row, effort } of entries) targets[effort] ??= row.id;
      const aliases = Object.fromEntries(Object.entries(variants.aliases || {}).filter(([alias, effort]) => targets[effort] && !targets[alias]));
      const variantIds = entries.map(({ row }) => row.id);
      const group = { ...selected.row, id, name, variantIds, reasoning_models: {
        label: name, models: targets, default_effort: selected.effort, follow_request: true,
        aliases, labels: variants.labels || {}, remove: variants.remove || [],
      } };
      for (const model of [id, ...variantIds]) grouped.set(model, group);
    }
  }
  return [...new Set(models.map(row => grouped.get(row.id) || row))];
}
