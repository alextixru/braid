//! Тонкий слой над `hickory-proto`: разбор запроса, сборка ответов
//! (A, пустой с кодом), запрос A для упреждающего резолва, первая запись A.

use hickory_proto::op::{Edns, Message, MessageType, OpCode, Query as WireQuery, ResponseCode};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::{Name, RData, Record, RecordType};
use std::net::Ipv4Addr;

/// Максимальный размер UDP-ответа, который объявляем в EDNS.
const EDNS_PAYLOAD: u16 = 1232;

/// Разобранный запрос: ровно один вопрос, стандартный опкод.
#[derive(Debug, Clone)]
pub struct Query {
    pub id: u16,
    /// Имя в нижнем регистре без завершающей точки.
    pub name: String,
    pub qtype: RecordType,
    pub recursion_desired: bool,
    pub edns: bool,
    wire: WireQuery,
}

impl Query {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let msg = Message::from_vec(bytes).ok()?;
        if msg.metadata.message_type != MessageType::Query || msg.metadata.op_code != OpCode::Query
        {
            return None;
        }
        let [wire] = msg.queries.as_slice() else {
            return None;
        };
        let name = wire
            .name()
            .to_ascii()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        Some(Self {
            id: msg.metadata.id,
            name,
            qtype: wire.query_type(),
            recursion_desired: msg.metadata.recursion_desired,
            edns: msg.edns.is_some(),
            wire: wire.clone(),
        })
    }

    fn response(&self, rcode: ResponseCode) -> Message {
        let mut m = Message::new(self.id, MessageType::Response, OpCode::Query);
        m.metadata.recursion_desired = self.recursion_desired;
        m.metadata.recursion_available = true;
        m.metadata.response_code = rcode;
        m.add_query(self.wire.clone());
        if self.edns {
            let mut edns = Edns::new();
            edns.set_max_payload(EDNS_PAYLOAD);
            m.set_edns(edns);
        }
        m
    }

    /// Ответ с одной записью A.
    pub fn answer_a(&self, ip: Ipv4Addr, ttl: u32) -> Option<Vec<u8>> {
        let mut m = self.response(ResponseCode::NoError);
        m.add_answer(Record::from_rdata(
            self.wire.name().clone(),
            ttl,
            RData::A(A(ip)),
        ));
        m.to_vec().ok()
    }

    /// Ответ без записей: `NoError` — NODATA, `ServFail` — kill switch.
    pub fn answer_empty(&self, rcode: ResponseCode) -> Option<Vec<u8>> {
        self.response(rcode).to_vec().ok()
    }
}

/// Запрос A по имени — для упреждающего резолва настоящего адреса через туннель.
pub fn build_a_query(id: u16, name: &str) -> Option<Vec<u8>> {
    let mut fqdn = Name::from_ascii(name).ok()?;
    fqdn.set_fqdn(true);
    let mut m = Message::new(id, MessageType::Query, OpCode::Query);
    m.metadata.recursion_desired = true;
    m.add_query(WireQuery::query(fqdn, RecordType::A));
    m.to_vec().ok()
}

/// Первая запись A из ответа с ожидаемым идентификатором и кодом `NoError`.
pub fn first_a(response: &[u8], expect_id: u16) -> Option<(Ipv4Addr, u32)> {
    let m = Message::from_vec(response).ok()?;
    if m.metadata.message_type != MessageType::Response
        || m.metadata.id != expect_id
        || m.metadata.response_code != ResponseCode::NoError
    {
        return None;
    }
    m.answers.iter().find_map(|r| match &r.data {
        RData::A(a) => Some((a.0, r.ttl)),
        _ => None,
    })
}

/// Идентификатор транзакции из первых двух байт.
pub fn transaction_id(msg: &[u8]) -> Option<u16> {
    match msg {
        [a, b, ..] => Some(u16::from_be_bytes([*a, *b])),
        _ => None,
    }
}
