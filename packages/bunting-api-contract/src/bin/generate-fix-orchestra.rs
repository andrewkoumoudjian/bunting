#![forbid(unsafe_code)]
//! Generates the Bunting-owned FIX Orchestra overlay from the canonical JSON profile.

use serde::Deserialize;
use std::fmt::Write as _;

const PROFILE: &str = include_str!("../../../../schemas/fix/bunting.fixlatest.competition.v1.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    #[serde(rename = "profileVersion")]
    version: String,
    messages: Vec<Message>,
    extension_fields: Vec<ExtensionField>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    name: String,
    msg_type: String,
    standard: bool,
}

#[derive(Deserialize)]
struct ExtensionField {
    tag: u32,
    name: String,
    #[serde(rename = "type")]
    field_type: String,
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn render(profile: &Profile) -> Result<String, std::fmt::Error> {
    let mut output = String::new();
    writeln!(output, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(
        output,
        "<fixr:repository xmlns:fixr=\"http://fixprotocol.io/2020/orchestra/repository\" name=\"{}\" version=\"1.0\">",
        escape(&profile.version)
    )?;
    writeln!(output, "  <fixr:metadata>")?;
    writeln!(
        output,
        "    <dcterms:title xmlns:dcterms=\"http://purl.org/dc/terms/\">Bunting competition extensions</dcterms:title>"
    )?;
    writeln!(output, "  </fixr:metadata>")?;
    writeln!(output, "  <fixr:fields>")?;
    for field in &profile.extension_fields {
        let field_type = if field.field_type == "data" {
            "data"
        } else {
            "String"
        };
        writeln!(
            output,
            "    <fixr:field id=\"{}\" name=\"{}\" type=\"{}\"/>",
            field.tag,
            escape(&field.name),
            field_type
        )?;
    }
    writeln!(output, "  </fixr:fields>")?;
    writeln!(output, "  <fixr:messages>")?;
    for (index, message) in profile
        .messages
        .iter()
        .filter(|message| !message.standard)
        .enumerate()
    {
        writeln!(
            output,
            "    <fixr:message id=\"{}\" name=\"{}\" msgType=\"{}\" category=\"Bunting\"/>",
            9_001 + index,
            escape(&message.name),
            escape(&message.msg_type)
        )?;
    }
    writeln!(output, "  </fixr:messages>")?;
    writeln!(output, "</fixr:repository>")?;
    Ok(output)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile: Profile = serde_json::from_str(PROFILE)?;
    print!("{}", render(&profile)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_overlay_is_generated_from_the_profile() -> Result<(), Box<dyn std::error::Error>> {
        let profile: Profile = serde_json::from_str(PROFILE)?;
        assert_eq!(
            render(&profile)?,
            include_str!("../../../../schemas/fix/bunting.fixlatest.competition.v1.orchestra.xml")
        );
        Ok(())
    }
}
