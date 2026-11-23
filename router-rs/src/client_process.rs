use std::{
    collections::BTreeMap, ffi::CString, io, os::unix::ffi::OsStrExt, process::Command, ptr,
};

unsafe extern "C" {
    fn responsibility_spawnattrs_setdisclaim(
        attributes: *mut libc::posix_spawnattr_t,
        disclaim: libc::c_int,
    ) -> libc::c_int;
}

struct SpawnAttributes(libc::posix_spawnattr_t);

impl Drop for SpawnAttributes {
    fn drop(&mut self) {
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

fn checked(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

// 客户端继承终端和进程编号，文件权限由客户端自身身份承担。
// 此入口接收 configured_process 生成的参数与环境，标准输入输出和目录沿用当前进程。
pub fn exec(command: &Command) -> io::Error {
    match execute(command) {
        Err(error) => error,
        Ok(()) => unreachable!("POSIX_SPAWN_SETEXEC 成功后已进入客户端"),
    }
}

fn execute(command: &Command) -> io::Result<()> {
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| CString::new(argument.as_bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut environment = std::env::vars_os().collect::<BTreeMap<_, _>>();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            environment.insert(name.to_owned(), value.to_owned());
        } else {
            environment.remove(name);
        }
    }
    let environment = environment
        .into_iter()
        .map(|(name, value)| {
            let mut entry = name.into_encoded_bytes();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            CString::new(entry)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let argv = arguments
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(ptr::null_mut()))
        .collect::<Vec<_>>();
    let envp = environment
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(ptr::null_mut()))
        .collect::<Vec<_>>();
    let mut raw_attributes = ptr::null_mut();
    unsafe { checked(libc::posix_spawnattr_init(&mut raw_attributes))? };
    let mut attributes = SpawnAttributes(raw_attributes);
    unsafe {
        // 与 Command::exec 一致，客户端使用 SIGPIPE 的默认处理。
        let mut defaults = std::mem::zeroed();
        libc::sigemptyset(&mut defaults);
        libc::sigaddset(&mut defaults, libc::SIGPIPE);
        checked(libc::posix_spawnattr_setsigdefault(
            &mut attributes.0,
            &defaults,
        ))?;
        checked(libc::posix_spawnattr_setflags(
            &mut attributes.0,
            (libc::POSIX_SPAWN_SETEXEC | libc::POSIX_SPAWN_SETSIGDEF) as libc::c_short,
        ))?;
        checked(responsibility_spawnattrs_setdisclaim(&mut attributes.0, 1))?;
        checked(libc::posix_spawnp(
            ptr::null_mut(),
            arguments[0].as_ptr(),
            ptr::null(),
            &attributes.0,
            argv.as_ptr(),
            envp.as_ptr(),
        ))
    }
}
