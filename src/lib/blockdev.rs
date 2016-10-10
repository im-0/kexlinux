use std;

extern crate mnt;
extern crate tempdir;

const PATH_SYS: &'static str = "/sys/class/block";
const PATH_DEV: &'static str = "/dev";

const CMD_BLKID: &'static str = "blkid";
const CMD_MOUNT: &'static str = "mount";
const CMD_UMOUNT: &'static str = "umount";

// TODO: Detailed errors.
#[derive(Debug)]
pub struct BlockDevError {}

impl std::convert::From<std::io::Error> for BlockDevError {
    fn from(_: std::io::Error) -> BlockDevError { BlockDevError{} }
}

impl std::convert::From<std::num::ParseIntError> for BlockDevError {
    fn from(_: std::num::ParseIntError) -> BlockDevError { BlockDevError{} }
}

impl std::convert::From<std::string::FromUtf8Error> for BlockDevError {
    fn from(_: std::string::FromUtf8Error) -> BlockDevError { BlockDevError{} }
}

#[derive(Debug)]
pub struct BlockDev {
    pub path: std::path::PathBuf,
    pub name: String,
    pub dev_major: u8,
    pub dev_minor: u8,
    pub has_holders: bool,
    pub partitions: Vec<BlockDev>,
}

fn trim_spaces_and_newline(str: &str) -> &str {
    str.trim_matches(|c| " \t\r\n".find(c).is_some())
}

impl BlockDev {
    fn parse_u8(str: Option<&str>) -> Result<u8, BlockDevError> {
        Ok(try!(try!(str.ok_or(BlockDevError{})).parse::<u8>()))
    }

    fn get_major_minor(path: &std::path::PathBuf)
            -> Result<(u8, u8), BlockDevError> {
        let strs = {
            use std::io::prelude::*;
            let mut buf = String::new();
            let mut file = try!(std::fs::File::open(&path.join("dev")));
            try!(file.read_to_string(&mut buf));
            buf
        };
        let mut strs = trim_spaces_and_newline(&strs).splitn(2, ":");
        let major = try!(BlockDev::parse_u8(strs.next()));
        let minor = try!(BlockDev::parse_u8(strs.next()));
        assert!(strs.next() == None);

        Ok((major, minor))
    }

    fn check_holders(path: &std::path::PathBuf) -> Result<bool, BlockDevError> {
        let path = path.join("holders");
        Ok(match try!(path.read_dir()).next() {
            Some(v) => {
                try!(v);
                true
            },
            None => false,
        })
    }

    fn get_partitions(path: &std::path::PathBuf)
            -> Result<Vec<BlockDev>, BlockDevError> {
        use std::iter::FromIterator;
        Ok(Vec::from_iter(try!(path.read_dir()).filter_map(
            |entry| match entry {
                Ok(entry) => match entry.path().join("partition").exists() {
                    true => BlockDev::from_sys_path(entry.path()).ok(),
                    false => None,
                },
                Err(_) => None,
            })))
    }

    pub fn from_sys_path(path: std::path::PathBuf)
            -> Result<BlockDev, BlockDevError> {
        debug!("Trying to get information about block device from {:?}...",
               path);

        let (major, minor) = try!(BlockDev::get_major_minor(&path));
        let name = String::from(
            try!(
                try!(path.file_name().ok_or(BlockDevError{}))
                    .to_str().ok_or(BlockDevError{})));
        let dev = BlockDev{
            path: std::path::PathBuf::from(PATH_DEV).join(&name),
            name: name,
            dev_major: major,
            dev_minor: minor,
            has_holders: try!(BlockDev::check_holders(&path)),
            partitions: try!(BlockDev::get_partitions(&path)),
        };

        debug!("Found block device \"{}\"", dev.name);
        Ok(dev)
    }

    pub fn from_dev_path(path: std::path::PathBuf)
            -> Result<BlockDev, BlockDevError> {
        debug!("Trying to get information about block device from {:?}...",
               path);

        let (dev_major, dev_minor) = {
            use std::os::unix::fs::MetadataExt;
            let rdev = try!(path.metadata()).rdev();

            (((rdev & 0xFF00u64) >> 8) as u8,
             (rdev & 0x00FFu64) as u8)
        };
        debug!("Device {:?} (major: {}, minor: {})",
               path, dev_major, dev_minor);

        for dev in try!(BlockDevs::new()) {
            if (dev.dev_major == dev_major) && (dev.dev_minor == dev_minor) {
                return Ok(dev)
            }
        }

        error!("Unable to info in /sys for device {:?}", path);
        Err(BlockDevError{})
    }
}

pub struct BlockDevs {
    read_dir: std::fs::ReadDir,
}

impl BlockDevs {
    pub fn new() -> Result<BlockDevs, BlockDevError> {
        Ok(BlockDevs{
            read_dir: try!(std::path::PathBuf::from(PATH_SYS).read_dir()),
        })
    }

    fn next_dev(&mut self) -> Result<Option<BlockDev>, BlockDevError> {
        match self.read_dir.next() {
            Some(dir_entry) => {
                BlockDev::from_sys_path(try!(dir_entry).path()).map(|v| Some(v))
            },
            None => Ok(None),
        }
    }
}

impl Iterator for BlockDevs {
    type Item = BlockDev;

    fn next(&mut self) -> Option<BlockDev> {
        loop {
            match self.next_dev() {
                Ok(maybe_dev) => return maybe_dev,
                Err(_) => (),  // Try to get info about next block device.
            }
        }
    }
}

#[derive(Debug)]
pub struct FS {
    pub dev: BlockDev,
    pub fs_type: String,
}

impl FS {
    fn parse_blkid_output(out: String) -> Result<String, BlockDevError> {
        #[derive(Default)]
        struct BlkIDInfo {
            usage: Option<String>,
            fs_type: Option<String>,
        }

        impl BlkIDInfo {
            fn build(mut self, kv: &str) -> BlkIDInfo {
                let mut kv = kv.splitn(2, "=");
                if let Some(key) = kv.next() {
                    // TODO: Unescape.
                    if let Some(value) = kv.next() {
                        match key {
                            "USAGE" => self.usage   = Some(String::from(value)),
                            "TYPE"  => self.fs_type = Some(String::from(value)),
                            _ => {
                                debug!("blkid gave unexpected key \"{}\"", key)
                            },
                        }
                    }
                };
                self
            }
        }

        let info = trim_spaces_and_newline(&out).split('\n').fold(
            BlkIDInfo::default(),
            BlkIDInfo::build);

        if try!(info.usage.ok_or(BlockDevError{})) != "filesystem" {
            debug!("Not filesystem");
            Err(BlockDevError{})
        } else {
            info.fs_type.ok_or(BlockDevError{})
        }
    }

    fn get_fs_type(dev: &BlockDev) -> Result<String, BlockDevError> {
        debug!("Probing {:?} with blkid...", dev.path);

        let mut cmd = std::process::Command::new(CMD_BLKID);
        cmd.arg("-p");                            // Bypass cache.
        cmd.args(&["-o", "export"]);              // Output in KEY=value format.
        cmd.args(&["-s", "USAGE", "-s", "TYPE"]); // Show only these tags.
        cmd.arg(try!(dev.path.to_str().ok_or(BlockDevError{})));
        cmd.stdin(std::process::Stdio::null());

        let output = try!(cmd.output());
        match output.status.success() {
            true => {
                FS::parse_blkid_output(try!(String::from_utf8(output.stdout)))
            },

            false => {
                use std::os::unix::process::ExitStatusExt;
                debug!("blkid command ({:?}) failed with return code \
                        {:?} || signal {:?}", cmd, output.status.code(),
                       output.status.signal());
                debug!("stdout: \"{}\"",
                       String::from_utf8_lossy(&output.stdout));
                debug!("stderr: \"{}\"",
                       String::from_utf8_lossy(&output.stderr));
                Err(BlockDevError{})
            },
        }
    }

    pub fn from_dev(dev: BlockDev) -> Result<FS, BlockDevError> {
        if dev.has_holders {
            debug!("Block device \"{}\" has holders, skipping", dev.name);
            return Err(BlockDevError{})
        } else if !dev.partitions.is_empty() {
            debug!("Block device \"{}\" has partitions, skipping", dev.name);
            return Err(BlockDevError{})
        }

        let fs = FS{
            fs_type: try!(FS::get_fs_type(&dev)),
            dev: dev,
        };
        debug!("Found FS \"{}\" on device \"{}\"", fs.fs_type, fs.dev.name);
        Ok(fs)
    }
}

pub fn get_filesystems<BlockDevIter>(block_devs: BlockDevIter) -> Vec<FS>
        where BlockDevIter: Iterator<Item=BlockDev> {
    use std::iter::FromIterator;
    Vec::from_iter(block_devs.filter_map(
        |dev| FS::from_dev(dev).ok()))
}

pub struct Mount {
    temp_dir: Option<tempdir::TempDir>,
    mount_path: std::path::PathBuf,
}

impl Mount {
    fn call_mount(mut cmd: std::process::Command, mount_cmd: &str)
            -> Result<(), BlockDevError> {
        cmd.stdin(std::process::Stdio::null());
        let output = try!(cmd.output());
        match output.status.success() {
            true => Ok(()),
            false => {
                use std::os::unix::process::ExitStatusExt;
                error!("{} command ({:?}) failed with return code \
                        {:?} || signal {:?}", mount_cmd, cmd,
                       output.status.code(), output.status.signal());
                error!("stdout: \"{}\"",
                       String::from_utf8_lossy(&output.stdout));
                error!("stderr: \"{}\"",
                       String::from_utf8_lossy(&output.stderr));
                Err(BlockDevError{})
            },
        }
    }

    pub fn mount(fs: &FS) -> Result<Mount, BlockDevError> {
        match mnt::get_mount(&fs.dev.path) {
            Ok(Some(existing_mount)) => {
                debug!("{:?} already mounted, will use existing mount",
                       fs.dev.path);
                Ok(Mount{
                    temp_dir: None,
                    mount_path: existing_mount.file,
                })
            },

            Ok(None) | Err(_) => {
                let temp_dir = try!(tempdir::TempDir::new("kexlinux"));
                let mount_path = temp_dir.path().join("mount");
                try!(std::fs::create_dir(&mount_path));

                debug!("Trying to mount {:?} on {:?}...", fs.dev.path, mount_path);

                let mut cmd = std::process::Command::new(CMD_MOUNT);
                cmd.args(&["-t", &fs.fs_type]);
                cmd.args(&["-o", "ro"]);        // Mount read-only.
                cmd.arg(try!(fs.dev.path.to_str().ok_or(BlockDevError{})));
                cmd.arg(try!(mount_path.to_str().ok_or(BlockDevError{})));
                try!(Mount::call_mount(cmd, "mount"));

                Ok(Mount{
                    temp_dir: Some(temp_dir),
                    mount_path: mount_path,
                })
            },
        }
    }

    fn try_umount(&self) -> Result<(), BlockDevError> {
        debug!("Trying to unmount {:?}...", self.mount_path);

        let mut cmd = std::process::Command::new(CMD_UMOUNT);
        cmd.arg(try!(self.mount_path.to_str().ok_or(BlockDevError{})));
        Mount::call_mount(cmd, "umount")
    }

    pub fn umount(&mut self) -> Result<(), BlockDevError> {
        if self.temp_dir.is_some() {
            let temp_dir =  std::mem::replace(&mut self.temp_dir, None);
            match self.try_umount() {
                Ok(v) => Ok(v),
                Err(v) => {
                    error!("umount failed, temporary directory {:?} will \
                           not be removed",
                           temp_dir.as_ref().unwrap().path());

                    // Do not try to remove temporary directory.
                    temp_dir.unwrap().into_path();

                    Err(v)
                },
            }
        } else {
            Ok(())
        }
    }

    pub fn path(&self) -> &std::path::PathBuf {
        &self.mount_path
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        match self.umount() {
            Ok(_) => (),
            Err(_) => {
                error!("Unable to unmount block device");
            }
        }
    }
}

#[test]
fn it_works() {
    println!("DEVICE: {:#?}", get_filesystems(BlockDevs::new().unwrap()));
}
