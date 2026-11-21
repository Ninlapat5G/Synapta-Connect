//! รูปแบบข้อความ JSON-RPC 2.0 และชนิด error ที่ทุกโมดูลใช้ร่วมกัน

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// error ที่ส่งกลับ client — `code` ตามตารางใน docs/API.md
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

pub type Res<T> = Result<T, RpcError>;

pub const INVALID_PARAMS: i32 = -32602;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INTERNAL: i32 = -32000;
pub const NOT_AVAILABLE: i32 = -32001; // เรียกผิดช่องทาง (เช่น pty ผ่าน HTTP)
pub const CONFLICT: i32 = -32002; // สถานะไม่ตรง เช่น offset ผิด, checksum ไม่ตรง
pub const BUSY: i32 = -32003; // ทรัพยากรเต็ม เช่น stream พร้อมกันครบแล้ว

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> RpcError {
        RpcError::new(INVALID_PARAMS, message)
    }
}

// ให้ `?` ใช้กับ io::Error / anyhow / serde_json ได้ตรง ๆ (RpcError ไม่ implement Error
// เพื่อไม่ให้ impl นี้ชนกับ From<T> for T)
impl<E: std::fmt::Display> From<E> for RpcError {
    fn from(e: E) -> Self {
        RpcError::new(INTERNAL, e.to_string())
    }
}

/// แปลง params เป็นชนิดที่ method ต้องการ
pub fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Res<T> {
    // method ที่ไม่มี params ส่ง null มา — ให้ถือเป็น {}
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| RpcError::invalid(e.to_string()))
}

pub fn to_value<T: Serialize>(v: T) -> Res<Value> {
    Ok(serde_json::to_value(v)?)
}

pub fn response(id: Option<Value>, result: Res<Value>) -> Value {
    let id = id.unwrap_or(Value::Null);
    match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    }
}

pub fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}
