use std::fmt::Write;
#[derive(Debug, Clone, Default)]
pub struct UserInfo {
    pub name: String,
    pub global_name: Option<String>,
    pub nickname: Option<String>,
    pub id: u64,
    pub roles: Vec<String>,
    pub avatar_url: Option<String>,
}

impl UserInfo {
    pub fn format_for_prompt(&self) -> String {
        let mut out = String::with_capacity(
            self.name.len()
                + self.global_name.as_ref().map_or(0, |s| s.len())
                + self.nickname.as_ref().map_or(0, |s| s.len())
                + self.avatar_url.as_ref().map_or(0, |s| s.len())
                + self.roles.iter().map(String::len).sum::<usize>()
                + 96,
        );

        out.push_str("[User Info]\nUsername: ");
        out.push_str(&self.name);

        if let Some(ref gn) = self.global_name {
            out.push_str("\nDisplay name: ");
            out.push_str(gn);
        }
        if let Some(ref nick) = self.nickname {
            out.push_str("\nServer nickname: ");
            out.push_str(nick);
        }

        out.push_str("\nID: ");
        let _ = write!(out, "{}", self.id);

        if !self.roles.is_empty() {
            out.push_str("\nRoles: ");
            for (i, role) in self.roles.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(role);
            }
        }
        if let Some(ref url) = self.avatar_url {
            out.push_str("\nAvatar: ");
            out.push_str(url);
        }

        out
    }
}
