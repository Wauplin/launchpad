//! Addresses the companion opens in the browser (docs/links.md). Every one carries `ws=` so the browser window follows
//! the companion's workspace; none ever carries a token.

use url::Url;

use crate::model::Session;

pub struct Links {
    origin: String,
    ws: String,
}

impl Links {
    pub fn new(origin: &str, ws: &str) -> Links {
        Links { origin: origin.trim_end_matches('/').to_string(), ws: ws.to_string() }
    }

    /// `<origin><path>` with the query pairs and `ws=` added.
    pub fn page(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut url = Url::parse(&format!("{}{path}", self.origin)).unwrap_or_else(|_| Url::parse("http://invalid/").unwrap());
        if !query.is_empty() || !self.ws.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
            if !self.ws.is_empty() {
                q.append_pair("ws", &self.ws);
            }
        }
        url.to_string()
    }

    /// Today, or one of its cards (`/?card=<item id>`).
    pub fn today_item(&self, id: &str) -> String {
        self.page("/", &[("card", id)])
    }

    pub fn today(&self) -> String {
        self.page("/", &[])
    }

    /// A session: its bot's chat at its thread for a bot session, else the agent in Agents.
    pub fn session(&self, s: &Session) -> String {
        match &s.launch_spec.bot {
            Some(bot) => match bot.thread.as_deref().filter(|t| !t.is_empty()) {
                Some(thread) => self.page(&format!("/bots/{}", enc(&bot.id)), &[("thread", thread)]),
                None => self.bot(&bot.id),
            },
            None => self.page("/agents", &[("session", &s.id)]),
        }
    }

    pub fn bot(&self, id: &str) -> String {
        self.page(&format!("/bots/{}", enc(id)), &[])
    }

    pub fn agents(&self) -> String {
        self.page("/agents", &[])
    }

    pub fn chats(&self) -> String {
        self.page("/chats", &[])
    }

    /// Settings → Devices, where the owner approves a pairing code.
    pub fn devices(&self) -> String {
        self.page("/settings/devices", &[])
    }
}

fn enc(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_carry_the_workspace() {
        let l = Links::new("http://127.0.0.1:7317/", "w 1");
        assert_eq!(l.today(), "http://127.0.0.1:7317/?ws=w+1");
        assert_eq!(l.today_item("session:a"), "http://127.0.0.1:7317/?card=session%3Aa&ws=w+1");
        let mut s: Session = serde_json::from_value(serde_json::json!({"id": "s1"})).unwrap();
        assert_eq!(l.session(&s), "http://127.0.0.1:7317/agents?session=s1&ws=w+1");
        s.launch_spec.bot = Some(serde_json::from_value(serde_json::json!({"id": "b/1", "thread": "t1"})).unwrap());
        assert_eq!(l.session(&s), "http://127.0.0.1:7317/bots/b%2F1?thread=t1&ws=w+1");
        assert_eq!(Links::new("http://h", "").today(), "http://h/");
    }
}
