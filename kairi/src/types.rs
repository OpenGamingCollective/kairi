use std::fmt::Display;

#[derive(Debug, poise::ChoiceParameter, PartialEq, Hash, Eq, Clone, Copy)]
pub enum MailingListEnum {
    #[name = "announcements"]
    Announcements,
    #[name = "governance"]
    Governance,
    #[name = "conventions"]
    Conventions,
    #[name = "kernel"]
    Kernel,
    #[name = "kernel-unstable"]
    KernelUnstable,
    #[name = "asusctl"]
    Asusctl,
}

impl Display for MailingListEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Announcements => write!(f, "announcements"),
            Self::Governance => write!(f, "governance"),
            Self::Conventions => write!(f, "conventions"),
            Self::Kernel => write!(f, "kernel"),
            Self::KernelUnstable => write!(f, "kernel-unstable"),
            Self::Asusctl => write!(f, "asusctl"),
        }
    }
}
