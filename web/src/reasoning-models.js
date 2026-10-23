export function groupReasoningModels(models, settings) {
  const variants = settings?.reasoning_variants;
  if (!variants) return models;
  const suffixes = Object.entries(variants.suffixes).filter(([,suffix])=>suffix).sort((a,b)=>b[1].length-a[1].length);
  const families = new Map();
  for (const row of models) {
    const match = suffixes.find(([,suffix])=>row.id.endsWith(suffix));
    if (!match) continue;
    const [effort,suffix] = match;
    const family = row.id.slice(0,-suffix.length);
    if (!families.has(family)) families.set(family,[]);
    families.get(family).push({row,effort});
  }
  const grouped = new Map();
  for (const entries of families.values()) {
    if (entries.length < 2) continue;
    const selected = entries.find(entry=>entry.effort===variants.default_effort) || entries[0];
    const suffix = variants.name_suffixes[selected.effort];
    const name = suffix && selected.row.name.endsWith(suffix) ? selected.row.name.slice(0,-suffix.length) : selected.row.name;
    const targets = Object.fromEntries(entries.map(({row,effort})=>[effort,row.id]));
    const aliases = Object.fromEntries(Object.entries(variants.aliases || {}).filter(([alias,effort])=>targets[effort] && !targets[alias]));
    const group = {...selected.row,name,reasoning_models:{label:name,models:targets,default_effort:selected.effort,follow_request:true,aliases,labels:variants.labels || {},remove:variants.remove || []}};
    for (const {row} of entries) grouped.set(row.id,group);
  }
  return [...new Set(models.map(row=>grouped.get(row.id) || row))];
}
