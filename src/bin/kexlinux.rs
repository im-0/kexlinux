#[macro_use] extern crate log;

extern crate clap;
extern crate env_logger;

extern crate kexlinux;
extern crate syslinux_conf;

fn kexlinux_from_mount(matches: &clap::ArgMatches)
        -> Result<kexlinux::KexLinux, kexlinux::KexLinuxError> {
    let root_dir = matches.value_of("ROOT DIR").unwrap();
    let root_dir = std::path::PathBuf::from(root_dir);

    match matches.value_of("CONF FILE PATH") {
        Some(conf_path) => {
            let conf_path = std::path::PathBuf::from(conf_path);
            kexlinux::KexLinux::from_local_conf_file_path(
                root_dir, conf_path)
        }

        None => {
            match matches.value_of("type") {
                Some(conf_type) => {
                    let conf_type = match conf_type {
                        "syslinux" => syslinux_conf::LocalConfType::SysLinux,
                        "isolinux" => syslinux_conf::LocalConfType::IsoLinux,
                        "extlinux" => syslinux_conf::LocalConfType::ExtLinux,
                        _ => panic!("This will never happen"),
                    };
                    kexlinux::KexLinux::from_local_type(root_dir, conf_type)
                }

                None => {
                    kexlinux::KexLinux::from_local(root_dir)
                }
            }
        }
    }
}

fn main() {
    env_logger::init().unwrap();

    let matches = clap::App::new("kexlinux")
        .about("Userspace bootloader for Linux using kexec")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand(clap::SubCommand::with_name("mount")
            .about("Boot from already mounted boot device.")
            .arg(clap::Arg::with_name("type")
                .help("Type of syslinux configuration. Only for autodetect.")
                .short("t")
                .long("type")
                .value_name("TYPE")
                .takes_value(true)
                .possible_values(&["syslinux", "isolinux", "extlinux"]))
            .arg(clap::Arg::with_name("ROOT DIR")
                .help("Path to the root directory of the boot device.")
                .required(true)
                .index(1))
            .arg(clap::Arg::with_name("CONF FILE PATH")
                .help("Path to the configuration file. Will be autodetected if \
                       omitted.")
                .index(2))
            .group(clap::ArgGroup::with_name("detection")
                .arg("type")
                .arg("CONF FILE PATH")))
        .get_matches();

    let kexlinux = if let Some(matches) = matches.subcommand_matches("mount") {
        kexlinux_from_mount(matches)
    } else {
        error!("No command");
        std::process::exit(1)
    };

    let kexlinux = match kexlinux {
        Ok(kexlinux) => kexlinux,
        Err(_) => {
            // TODO: Log actual reason.
            error!("Unable to initialize kexlinux");
            std::process::exit(1)
        },
    };

    if let Err(_) = kexlinux::KexLinux::boot(&kexlinux.get_conf().ontimeout) {
        // TODO: Log actual reason.
        error!("Unable to kexec");
        std::process::exit(1)
    }
}
