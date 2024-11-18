use tailcall_valid::{Valid, ValidationError, Validator};

use crate::core::config::{LinkConfig, LinkType};
use crate::core::directive::DirectiveCodec;

pub struct Links;

impl TryFrom<Vec<LinkConfig>> for Links {
    type Error = ValidationError<String>;

    fn try_from(links: Vec<LinkConfig>) -> Result<Self, Self::Error> {
        Valid::from_iter(links.iter().enumerate(), |(pos, link)| {
            Valid::succeed(link.to_owned())
                .and_then(|link| {
                    if link.src.is_empty() {
                        Valid::fail("Link src cannot be empty".to_string())
                    } else {
                        Valid::succeed(link)
                    }
                })
                .and_then(|link| {
                    if let Some(id) = &link.id {
                        if links.iter().filter(|l| l.id.as_ref() == Some(id)).count() > 1 {
                            return Valid::fail(format!("Duplicated id: {}", id));
                        }
                    }
                    Valid::succeed(link)
                })
                .trace(&pos.to_string())
        })
        .and_then(|links| {
            let script_links = links
                .iter()
                .filter(|l| l.type_of == LinkType::Script)
                .collect::<Vec<&LinkConfig>>();

            if script_links.len() > 1 {
                Valid::fail("Only one script link is allowed".to_string())
            } else {
                Valid::succeed(links)
            }
        })
        .and_then(|links| {
            let key_links = links
                .iter()
                .filter(|l| l.type_of == LinkType::Key)
                .collect::<Vec<&LinkConfig>>();

            if key_links.len() > 1 {
                Valid::fail("Only one key link is allowed".to_string())
            } else {
                Valid::succeed(links)
            }
        })
        .trace(LinkConfig::trace_name().as_str())
        .trace("schema")
        .map_to(Links)
        .to_result()
    }
}
