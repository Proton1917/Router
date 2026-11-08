const invoke = window.__TAURI__.core.invoke;
const byId = id => document.getElementById(id);
let info;
let busy = false;
let accounts = [{id:'primary',label:'主账号',directory:''},{id:'backup',label:'备用账号',directory:''}];

function selection() {
  return {client:byId('client').value,provider:byId('provider').value,base_url:byId('base-url').value.trim(),key:byId('key').value,model:byId('model').value.trim(),command:byId('command').value.trim(),install_dependencies:byId('install').checked,authentication:byId('authentication').value,accounts};
}

function renderAccounts() {
  byId('accounts').replaceChildren(...accounts.map((account,index) => {
    const box = document.createElement('div');
    const label = document.createElement('label'); label.textContent = index === 0 ? '主账号名称' : `备用账号 ${index} 名称`;
    const name = document.createElement('input'); name.value = account.label; name.oninput = () => { account.label = name.value; }; label.append(name);
    const pathLabel = document.createElement('label'); pathLabel.textContent = '账号配置目录';
    const path = document.createElement('input'); path.value = account.directory; path.oninput = () => { account.directory = path.value; }; pathLabel.append(path);
    const details = document.createElement('details'); const summary = document.createElement('summary'); summary.textContent = '账号配置目录'; details.append(summary,pathLabel);
    box.append(label,details); return box;
  }));
}

function accountDirectories() {
  for (const [index,account] of accounts.entries()) account.directory = index === 0 && byId('existing-account').checked ? info.default_account_directory : `${byId('directory').value}/credentials/subscriptions/${account.id}`;
  renderAccounts();
}

function authenticationChanged() {
  const subscription = byId('authentication').value === 'subscription';
  for (const id of ['api-fields','load-models','model-list-field','model-field']) byId(id).hidden = subscription;
  byId('subscription-fields').hidden = !subscription;
  byId('key').required = !subscription; byId('model').required = !subscription;
  byId('model-hint').textContent = subscription ? '模型与思考强度在官方 Claude Code 中选择。' : 'Router 会自动配置接口协议和认证方式。';
}

async function action(name, args = {}, progress = '正在打开配置…') {
  if (busy) return;
  busy = true;
  byId('error').hidden = true;
  document.querySelectorAll('button,input,select').forEach(element => element.disabled = true);
  byId('progress').textContent = progress;
  try {
    const result = await invoke(name, args);
    byId('progress').textContent = '';
    return result;
  } catch (error) {
    byId('error').textContent = String(error);
    byId('error').hidden = false;
    byId('progress').textContent = '';
    byId('error').scrollIntoView({block:'nearest'});
  } finally {
    busy = false;
    document.querySelectorAll('button,input,select').forEach(element => element.disabled = false);
  }
}

function providerChanged() {
  const provider = info.recipes.providers[byId('provider').value];
  byId('base-url').value = provider.base_url;
  byId('address-field').hidden = !!provider.base_url;
  byId('key').value = '';
  byId('model').value = '';
  byId('model-list').replaceChildren(new Option('读取模型后选择，或在下方填写', ''));
  byId('model-hint').textContent = 'Router 会自动配置接口协议和认证方式。';
}

function clientChanged() {
  const client = info.recipes.clients[byId('client').value];
  byId('command').value = client.command;
  byId('provider').replaceChildren(...Object.entries(info.recipes.providers).filter(([,provider]) => provider.protocols.includes(client.protocol)).map(([id,provider]) => new Option(provider.label,id)));
  renderEnvironment();
  providerChanged();
  byId('auth-field').hidden = !client.subscription_template;
  if (!client.subscription_template) byId('authentication').value = 'api';
  authenticationChanged();
}

function renderEnvironment() {
  const client = info.recipes.clients[byId('client').value];
  const missing = [client.program,'python3'].filter(name => !info.environment[name]);
  const help = missing.length > 0 && !info.environment.brew;
  byId('environment').textContent = missing.length ? `需要安装：${missing.join('、')}。${info.environment.brew ? '创建时按自动安装选项准备。' : '安装 Homebrew 后可自动准备，也可自行安装这些程序。'}` : `已找到 ${client.label} 与 Python 3，程序路径将自动配置。`;
  byId('install-field').hidden = !missing.length || !info.environment.brew;
  byId('install').checked = !!info.environment.brew;
  byId('dependency-help').hidden = !help;
  byId('dependency-hint').hidden = !help;
}

function installArguments(withSelection) {
  return {directory:byId('directory').value,launchers:byId('launchers').value,gateway:byId('gateway').value,control:byId('control').value,shell:byId('shell').checked,selection:withSelection ? selection() : null};
}

byId('client').onchange = clientChanged;
byId('authentication').onchange = authenticationChanged;
byId('existing-account').onchange = accountDirectories;
byId('directory').onchange = accountDirectories;
byId('add-account').onclick = () => { const id = `backup-${crypto.randomUUID().slice(0,8)}`; accounts.push({id,label:`备用账号 ${accounts.length}`,directory:`${byId('directory').value}/credentials/subscriptions/${id}`}); renderAccounts(); };
byId('provider').onchange = providerChanged;
byId('choose').onclick = () => action('choose_config');
byId('resume').onclick = () => action('open_saved');
byId('update').onclick = () => action('update_installation', {}, '正在准备更新…');
byId('dependency-help').onclick = () => action('dependency_help', {}, '正在打开官方安装页面…');
byId('refresh-environment').onclick = async () => {
  const result = await action('setup_info', {}, '正在查询本机程序…');
  if (result) { info.environment = result.environment; renderEnvironment(); }
};
byId('model-list').onchange = () => { byId('model').value = byId('model-list').value; };
byId('load-models').onclick = async () => {
  if (!byId('key').reportValidity() || !byId('base-url').value.trim()) return;
  const result = await action('setup_models', {selection:selection()}, '正在读取服务商模型目录…');
  if (!result) return;
  byId('model-list').replaceChildren(new Option('请选择模型', ''), ...result.models.map(model => new Option(`${model.name}${model.name === model.id ? '' : ` · ${model.id}`}`, model.id)));
  byId('model-hint').textContent = `已读取 ${result.models.length} 个模型。选择后创建命令，实际调用权限由服务商确认。`;
  byId('model-list').focus();
};
byId('create').onsubmit = event => { event.preventDefault(); action('create_config', {request:installArguments(true)}, '正在准备安装…'); };
byId('empty').onclick = () => action('create_config', {request:installArguments(false)}, '正在创建配置…');

window.__TAURI__.event.listen('setup-progress', event => { byId('progress').textContent = event.payload; });
invoke('setup_info').then(result => {
  info = result;
  for (const id of ['directory','launchers','gateway','control']) byId(id).value = info[id];
  byId('client').replaceChildren(...Object.entries(info.recipes.clients).map(([id,client]) => new Option(client.label,id)));
  clientChanged();
  accountDirectories();
  byId('resume').hidden = !info.saved && !info.update;
  if (info.update) {
    byId('heading').textContent = '更新 Router 组件';
    byId('description').textContent = '新版应用已就绪，一次更新所有运行资源。';
    byId('update-panel').hidden = false;
    byId('create').hidden = true;
    byId('resume').textContent = '稍后更新，打开控制台';
  }
  if (info.error) { byId('error').textContent = info.error; byId('error').hidden = false; }
}).catch(error => { byId('error').textContent = String(error); byId('error').hidden = false; });
