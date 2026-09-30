const invoke = window.__TAURI__.core.invoke;
const byId = id => document.getElementById(id);
async function action(name, args = {}) {
  byId('error').hidden = true;
  document.querySelectorAll('button').forEach(button => button.disabled = true);
  byId('progress').textContent = '正在打开配置…';
  try { await invoke(name, args); byId('progress').textContent = ''; }
  catch (error) { byId('error').textContent = String(error); byId('error').hidden = false; byId('progress').textContent = ''; }
  finally { document.querySelectorAll('button').forEach(button => button.disabled = false); }
}
byId('choose').onclick = () => action('choose_config');
byId('resume').onclick = () => action('open_saved');
byId('create').onsubmit = event => { event.preventDefault(); action('create_config', { directory: byId('directory').value, launchers: byId('launchers').value, gateway: byId('gateway').value, control: byId('control').value, shell: byId('shell').checked }); };
invoke('setup_info').then(info => {
  byId('directory').value = info.directory;
  byId('launchers').value = info.launchers;
  byId('gateway').value = info.gateway;
  byId('control').value = info.control;
  byId('resume').hidden = !info.saved;
  if (info.error) { byId('error').textContent = info.error; byId('error').hidden = false; }
}).catch(error => { byId('error').textContent = String(error); byId('error').hidden = false; });
