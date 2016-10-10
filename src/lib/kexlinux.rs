use std;

extern crate natord;
extern crate syslinux_conf;

use blockdev;

const CMD_KEXEC: &'static str = "kexec";

#[derive(Debug)]
pub struct SyslinuxConf {
    pub timeout: Option<f64>,
    pub total_timeout: Option<f64>,

    pub ontimeout: syslinux_conf::Label,
    pub onerror: Option<syslinux_conf::Label>,

    pub default_name: Option<String>,
    pub labels: syslinux_conf::Labels,
}

#[derive(Debug)]
pub struct KexLinux {
    reader: syslinux_conf::Reader,
    conf: SyslinuxConf,
}

// TODO: Detailed errors.
#[derive(Debug)]
pub struct KexLinuxError {}

impl std::convert::From<syslinux_conf::ReaderError> for KexLinuxError {
    fn from(_: syslinux_conf::ReaderError) -> KexLinuxError { KexLinuxError{} }
}

impl std::convert::From<std::io::Error> for KexLinuxError {
    fn from(_: std::io::Error) -> KexLinuxError { KexLinuxError{} }
}

impl std::convert::From<blockdev::BlockDevError> for KexLinuxError {
    fn from(_: blockdev::BlockDevError) -> KexLinuxError { KexLinuxError{} }
}

impl SyslinuxConf {
    fn fix_append(mut label: syslinux_conf::Label) -> syslinux_conf::Label {
        match label.kernel_or_config {
            syslinux_conf::KernelOrConfig::Kernel(ref mut kernel) => {
                kernel.append = kernel.append.clone().and_then(
                    |v| if v == "-" { None } else { Some(v) })
            },
        };
        label
    }

    fn filter_map_labels(label_defaults: syslinux_conf::Label,
                     labels: syslinux_conf::Labels) -> syslinux_conf::Labels {
        use std::iter::FromIterator;
        syslinux_conf::Labels::from_iter(labels.into_iter()
            .map(
                |(label_name, label)| {
                    use kexlinux::syslinux_conf::ApplyDefaults;
                    (label_name, SyslinuxConf::fix_append(
                        label.apply_defaults(&label_defaults)))
                })
            .filter(
                |&(ref label_name, ref label)| {
                    match label.kernel_or_config {
                        syslinux_conf::KernelOrConfig::Kernel(ref kernel) => {
                            match kernel.kernel_file {
                                Some(syslinux_conf::KernelFile::Linux(_)) => {
                                    true
                                },

                                // Other kernel types are not supported.
                                Some(ref kernel_file) => {
                                    warn!(
                                        "Unsupported kernel type {:?} in \
                                        \"{}\", skipping",
                                        kernel_file, label_name);
                                    false
                                },

                                None => {
                                    warn!(
                                        "No kernel in \"{}\", skipping",
                                        label_name);
                                    false
                                },
                            }
                        },
                    }
            }))
    }

    fn from_conf(conf: syslinux_conf::SyslinuxConf)
            -> Result<SyslinuxConf, KexLinuxError> {
        let labels = SyslinuxConf::filter_map_labels(
            conf.global.label_defaults, conf.labels);

        let default = conf.global.default.as_ref().and_then(
            |default_name| labels.get(default_name).cloned());
        let ontimeout = conf.global.ontimeout.and_then(
            |ontimeout_name| labels.get(&ontimeout_name).cloned());

        let default_name = match default {
            Some(_) => conf.global.default,
            None => {
                warn!("Default label not found: \"{:?}\"", conf.global.default);
                None
            },
        };

        Ok(SyslinuxConf{
            timeout: conf.global.timeout,
            total_timeout: conf.global.total_timeout,

            ontimeout: try!(ontimeout.or(
                default.or_else(|| match labels.front() {
                    Some((_, first_label)) => Some(first_label.clone()),
                    None => {
                        error!("Nothing to boot");
                        None
                    },
                })).ok_or(KexLinuxError{})),
            onerror: conf.global.onerror.and_then(
                |onerror_name| labels.get(&onerror_name).cloned()),

            default_name: default_name,
            labels: labels,
        })
    }
}

impl KexLinux {
    fn from_reader(reader: syslinux_conf::Reader)
            -> Result<KexLinux, KexLinuxError> {
        Ok(KexLinux{
            conf: try!(SyslinuxConf::from_conf(try!(reader.read()))),
            reader: reader,
        })
    }

    pub fn from_local_conf_file_path(root: std::path::PathBuf,
                                     conf_file_path: std::path::PathBuf)
            -> Result<KexLinux, KexLinuxError> {
        KexLinux::from_reader(try!(
            syslinux_conf::Reader::from_local_conf_file_path(root,
                                                             conf_file_path)))
    }

    pub fn from_local_type(root: std::path::PathBuf,
                           local_type: syslinux_conf::LocalConfType)
            -> Result<KexLinux, KexLinuxError> {
        KexLinux::from_reader(try!(
            syslinux_conf::Reader::from_local_type(root, local_type)))
    }

    pub fn from_local(root: std::path::PathBuf)
            -> Result<KexLinux, KexLinuxError> {
        KexLinux::from_reader(try!(
            syslinux_conf::Reader::from_local(root)))
    }

    fn from_device_list<BlockDevIter>(devs: BlockDevIter)
            -> Result<KexLinux, KexLinuxError>
            where BlockDevIter: Iterator<Item=blockdev::BlockDev> {
        let mut filesystems = blockdev::get_filesystems(devs);
        filesystems.sort_by(|a, b| natord::compare(&a.dev.name, &b.dev.name));

        for fs in filesystems {
            match blockdev::Mount::mount(&fs) {
                Ok(fs) => match KexLinux::from_local(fs.path().clone()) {
                    Ok(kexlinux) => return Ok(kexlinux),
                    Err(_) => (),  // continue
                },

                Err(_) => (),  // continue
            }
        };

        error!("Unable to find bootable block device");
        Err(KexLinuxError{})
    }

    pub fn from_device_path(dev: std::path::PathBuf)
            -> Result<KexLinux, KexLinuxError> {
        let dev = try!(blockdev::BlockDev::from_dev_path(dev));
        match dev.partitions.is_empty() {
            true => KexLinux::from_device_list(vec![dev].into_iter()),
            false => KexLinux::from_device_list(dev.partitions.into_iter()),
        }
    }

    pub fn auto() -> Result<KexLinux, KexLinuxError> {
        KexLinux::from_device_list(try!(blockdev::BlockDevs::new()))
    }

    pub fn get_conf(&self) -> &SyslinuxConf {
        &self.conf
    }

    fn check_kexec_output(mut cmd: std::process::Command, stage: &str)
            -> Result<(), KexLinuxError> {
        let output = try!(cmd.output());
        match output.status.success() {
            true => Ok(()),
            false => {
                use std::os::unix::process::ExitStatusExt;
                error!("kexec ({}) command ({:?}) failed with return code \
                        {:?} || signal {:?}", stage, cmd, output.status.code(),
                       output.status.signal());
                error!("stdout: \"{}\"",
                       String::from_utf8_lossy(&output.stdout));
                error!("stderr: \"{}\"",
                       String::from_utf8_lossy(&output.stderr));
                Err(KexLinuxError{})
            },
        }
    }

    fn load_kernel(label: &syslinux_conf::Label) -> Result<(), KexLinuxError> {
        let kernel = match label.kernel_or_config {
            syslinux_conf::KernelOrConfig::Kernel(ref kernel) => kernel,
        };

        let kernel_file = match kernel.kernel_file {
            Some(syslinux_conf::KernelFile::Linux(ref kernel_file)) => {
                kernel_file
            },

            Some(ref kernel_file) => {
                error!("Unsupported kernel type {:?}, unable to kexec",
                       kernel_file);
                return Err(KexLinuxError{});
            },

            None => {
                error!("No kernel, unable to kexec");
                return Err(KexLinuxError{});
            },
        };

        let mut cmd = std::process::Command::new(CMD_KEXEC);

        info!("Loading kernel \"{}\"...", kernel_file.to_string_lossy());
        cmd.args(
            &["--load", try!(kernel_file.to_str().ok_or(KexLinuxError{}))]);

        if let Some(ref initrd) = kernel.initrd {
            info!("With initrd: \"{}\"", initrd.to_string_lossy());
            cmd.args(
                &["--initrd", try!(initrd.to_str().ok_or(KexLinuxError{}))]);
        }

        if let Some(ref append) = kernel.append {
            info!("With append: \"{}\"", append);
            cmd.args(&["--append", append]);
        }

        cmd.stdin(std::process::Stdio::null());

        KexLinux::check_kexec_output(cmd, "load")
    }

    fn kexec() -> Result<(), KexLinuxError> {
        let mut cmd = std::process::Command::new(CMD_KEXEC);
        cmd.arg("--exec");
        cmd.stdin(std::process::Stdio::null());
        try!(KexLinux::check_kexec_output(cmd, "exec"));

        panic!("This will never happen")
    }

    pub fn boot(label: &syslinux_conf::Label) -> Result<(), KexLinuxError> {
        KexLinux::load_kernel(label).and_then(|_| KexLinux::kexec())
    }
}
