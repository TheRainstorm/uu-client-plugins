//! UU wire identifiers end here; ownership and injection stay in the engine.
use anyhow::{ensure,Result};
use serde_json::Value;
use uu_core::native_input::{InputCommand as I,KeyCode,MouseButton};

pub(super) enum Message { Heartbeat, Commands(Vec<I>), Text(String) }
fn usage(vk:u64)->Option<u16> {
    let name=match vk {
        0x41..=0x5a=>format!("Key{}",char::from_u32(vk as u32)?),
        0x30..=0x39=>format!("Digit{}",char::from_u32(vk as u32)?),
        0x70..=0x87=>format!("F{}",vk-0x6f),
        0x60..=0x69=>format!("Numpad{}",vk-0x60),
        other=>match other {
            8=>"Backspace",9=>"Tab",13=>"Enter",19=>"Pause",20=>"CapsLock",27=>"Escape",32=>"Space",
            33=>"PageUp",34=>"PageDown",35=>"End",36=>"Home",37=>"ArrowLeft",38=>"ArrowUp",39=>"ArrowRight",40=>"ArrowDown",
            44=>"PrintScreen",45=>"Insert",46=>"Delete",91=>"MetaLeft",92=>"MetaRight",93=>"ContextMenu",
            106=>"NumpadMultiply",107=>"NumpadAdd",109=>"NumpadSubtract",110=>"NumpadDecimal",111=>"NumpadDivide",
            144=>"NumLock",145=>"ScrollLock",16|160=>"ShiftLeft",161=>"ShiftRight",17|162=>"ControlLeft",163=>"ControlRight",18|164=>"AltLeft",165=>"AltRight",
            186=>"Semicolon",187=>"Equal",188=>"Comma",189=>"Minus",190=>"Period",191=>"Slash",192=>"Backquote",
            219=>"BracketLeft",220=>"Backslash",221=>"BracketRight",222=>"Quote",226=>"IntlBackslash",
            _=>return None,
        }.into(),
    };
    KeyCode::from_w3c_code(&name).map(|k|k.0)
}
pub(super) fn parse(data:&[u8])->Result<Message> {
    ensure!(data.len()<=32*1024,"input message exceeds limit");
    let v:Value=serde_json::from_slice(data).map_err(|_|anyhow::anyhow!("invalid input JSON"))?;
    if v["heartbeat"]=="kbd_heartbeat" {return Ok(Message::Heartbeat);}
    ensure!(v.get("screen_id").is_none_or(|s|matches!(s.as_i64(),Some(-2..=0))),"input source was not advertised");
    let action=v["action"].as_str().ok_or_else(||anyhow::anyhow!("input action missing"))?;
    let delta=|key:&str|->Result<i16>{Ok(i16::try_from(v[key].as_i64().ok_or_else(||anyhow::anyhow!("invalid input delta"))?)?)};
    let axis=|key:&str|->Result<u16>{let x=v[key].as_f64().filter(|n|n.is_finite()&&(0.0..=1.0).contains(n)).ok_or_else(||anyhow::anyhow!("invalid input coordinate"))?;Ok((x*65535.).round() as u16)};
    if action=="text_input" {
        let text=v["content"].as_str().ok_or_else(||anyhow::anyhow!("text input missing"))?;
        ensure!(text.len()<=16*1024&&!text.contains('\0'),"invalid text input");
        return Ok(Message::Text(text.into()));
    }
    let command=match action {
        "kbd_press"|"kbd_release"|"kbd_click"=>I::Key {usage:usage(v["key"].as_u64().ok_or_else(||anyhow::anyhow!("invalid keyboard identifier"))?).ok_or_else(||anyhow::anyhow!("unsupported keyboard identifier"))?,pressed:action!="kbd_release"},
        "mouse_press"|"mouse_release"|"mouse_click"=>I::Button {button:match v["button"].as_u64(){Some(1)=>MouseButton::Left,Some(2)=>MouseButton::Right,Some(16)=>MouseButton::Middle,Some(32)=>MouseButton::Back,Some(64)=>MouseButton::Forward,_=>anyhow::bail!("unsupported mouse button")},pressed:action!="mouse_release"},
        "mouse_move_absolute"=>I::Absolute {x:axis("abs_x")?,y:axis("abs_y")?},
        "mouse_move_relative"=>I::Relative {dx:delta("delta_x")?,dy:delta("delta_y")?},
        "mouse_scroll"=>I::Wheel {dx:delta("delta_x")?,dy:delta("delta_y")?},
        _=>anyhow::bail!("unsupported input action"),
    };
    let mut commands=vec![command];
    if action.ends_with("_click") {commands.push(match command {I::Key{usage,..}=>I::Key{usage,pressed:false},I::Button{button,..}=>I::Button{button,pressed:false},_=>unreachable!()});}
    Ok(Message::Commands(commands))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_values_become_standard_hid_and_normalized_intents() {
        assert_eq!(usage(65),Some(4));assert_eq!(usage(165),Some(0xe6));assert_eq!(usage(0x87),Some(0x73));
        assert!(matches!(parse(br#"{"action":"mouse_move_absolute","abs_x":0.5,"abs_y":1}"#).unwrap(),Message::Commands(v) if matches!(v[0],I::Absolute{x:32768,y:65535})));
        assert!(matches!(parse(br#"{"action":"mouse_click","button":16}"#).unwrap(),Message::Commands(v) if matches!(v.as_slice(),[I::Button{button:MouseButton::Middle,pressed:true},I::Button{pressed:false,..}])));
        for value in [br#"{"action":"kbd_press","key":999}"#.as_slice(),br#"{"action":"mouse_move_absolute","abs_x":2,"abs_y":0}"#,br#"{"action":"shutdown_system"}"#] {assert!(parse(value).is_err());}
    }
}
