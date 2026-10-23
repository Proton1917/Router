import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, isAbsolute, resolve } from 'node:path';
import { customizeCommandProfile, isExclusiveCommandProfile, plannedProfileRouteCount, pruneDetachedCommandProfiles } from '../web/src/model-ownership.js';

const path = resolve(process.argv[2]);
const config = JSON.parse(readFileSync(path,'utf8'));
const manager = config.management;
const tokenPath = isAbsolute(manager.token_file) ? manager.token_file : resolve(dirname(path),manager.token_file);
const headers = {Authorization:`Bearer ${readFileSync(tokenPath,'utf8').trim()}`,'Content-Type':'application/json'};
const base = `http://${manager.listen}/api/`;
async function request(endpoint, body) {
  const response = await fetch(base + endpoint,{headers,method:body ? 'POST' : 'GET',body:body ? JSON.stringify(body) : undefined,signal:AbortSignal.timeout(30000)});
  const value = await response.json();
  if (!response.ok) throw new Error(`${endpoint}: ${JSON.stringify(value)}`);
  return value;
}
const state = await request('state');
assert.deepEqual(state.config,config,'指定配置与运行配置不一致');
async function preview(candidate) {
  const {config:planned} = await request('preview',{revision:state.revision,config:candidate});
  for (const profile of Object.keys(planned.profiles)) {
    assert.equal(plannedProfileRouteCount(candidate,profile),planned.routes.filter(route => route.profile === profile).length,`关联路由数量：${profile}`);
  }
  return planned;
}
// 校验用例按名称独立执行；生成用例重新排列后，内容仍应完整一致。
const orderedCases = value => ({...value,validation_cases:[...value.validation_cases].sort((a,b)=>a.name.localeCompare(b.name))});
assert.deepEqual(orderedCases(await preview(config)),orderedCases(config));
const entries = Object.entries(manager.commands).flatMap(([command,spec]) => (spec.model_bindings || []).map(binding => ({command,binding})));
const exclusive = entries.find(({command,binding}) => !binding.route_ids?.length && isExclusiveCommandProfile(config,binding.profile,command,binding.id));
assert.ok(exclusive,'实际配置需包含独享绑定');
const protocol = config.runtime.server.endpoints[manager.templates[manager.commands[exclusive.command].template].request_path];
const shared = entries.find(({command,binding}) => !isExclusiveCommandProfile(config,binding.profile,command,binding.id) && Object.values(config.profiles[binding.profile]).every(destination => !manager.backend_protocols[destination.target] || manager.backend_protocols[destination.target].includes(protocol)));
assert.ok(exclusive && shared,'实际配置需同时包含独享和共享绑定');
function edit(candidate, entry) {
  const previous = structuredClone(candidate.management.commands[entry.command]);
  const modified = customizeCommandProfile(candidate,entry.command,entry.binding.id,profile => {
    profile.fast = structuredClone(profile.fast || profile.standard);
    profile.fast.field_policy ||= {};
    profile.fast.field_policy.set ||= {};
    profile.fast.field_policy.set['/service_tier'] = 'priority';
  });
  candidate.profiles[modified.id] = modified.profile;
  candidate.management.commands[entry.command].model_bindings.find(item => item.id === entry.binding.id).profile = modified.id;
  pruneDetachedCommandProfiles(candidate,previous);
  return modified;
}
const single = structuredClone(config);
const first = edit(single,exclusive);
assert.equal(first.id,exclusive.binding.profile);
assert.equal(edit(single,exclusive).id,first.id,'连续编辑需保持标识');
const singlePlan = await preview(single);
assert.equal(Object.keys(singlePlan.profiles).length,Object.keys(config.profiles).length);
assert.deepEqual(singlePlan.profiles[first.id],first.profile);
const separate = structuredClone(config);
const copied = edit(separate,shared);
assert.notEqual(copied.id,shared.binding.profile);
assert.deepEqual(separate.profiles[shared.binding.profile],config.profiles[shared.binding.profile]);
assert.deepEqual(separate.management.integrations,config.management.integrations);
assert.equal(edit(separate,shared).id,copied.id,'连续编辑副本需保持标识');
const separatePlan = await preview(separate);
assert.deepEqual(separatePlan.profiles[shared.binding.profile],config.profiles[shared.binding.profile]);
assert.deepEqual(separatePlan.management.integrations,config.management.integrations);
for (const mode of ['remove','replace']) {
  const candidate = structuredClone(config);
  const command = candidate.management.commands[exclusive.command];
  const previous = structuredClone(command);
  if (mode === 'remove') {
    command.model_bindings = command.model_bindings.filter(item => item.id !== exclusive.binding.id);
    if (command.model === exclusive.binding.model) command.model = null;
  } else {
    command.model_bindings.find(item => item.id === exclusive.binding.id).profile = shared.binding.profile;
  }
  pruneDetachedCommandProfiles(candidate,previous);
  assert.ok(!candidate.profiles[exclusive.binding.profile]);
  const planned = await preview(candidate);
  assert.ok(!planned.profiles[exclusive.binding.profile]);
}
assert.equal((await request('state')).revision,state.revision,'检查期间配置发生变化');
console.log(JSON.stringify({ok:true,exclusive:exclusive.command,shared:shared.command,checks:['exclusive-edit','repeated-edit','shared-copy','remove-binding','replace-binding','planned-route-count','preview-without-save']}));
