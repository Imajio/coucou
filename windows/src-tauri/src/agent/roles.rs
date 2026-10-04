// The roles a session can take: who it is, how it works and which tools it
// has. The tool lists are enforced here, not just described to the model: a
// reviewer has no tool that writes.

use serde::Serialize;

pub struct RoleDef {
    pub id: &'static str,
    pub name: &'static str,
    /// The avatar's color.
    pub color: &'static str,
    /// One line for the role picker.
    pub summary: &'static str,
    pub prompt: &'static str,
    pub tools: &'static [&'static str],
}

const READ: [&str; 4] = ["read_file", "list_dir", "glob", "grep"];

macro_rules! tools {
    ($($t:expr),* $(,)?) => { &[ READ[0], READ[1], READ[2], READ[3], $($t),* ] };
}

pub const ROLES: &[RoleDef] = &[
    RoleDef {
        id: "engineer",
        name: "Engineer",
        color: "#4F8CFF",
        summary: "Writes and fixes code, runs the tests",
        prompt: "You are a senior software engineer. Understand the code before you change it: find the relevant files, read them, follow the project's conventions. Make focused changes that do what was asked and nothing more. After changing code, build it or run the tests when the project has them, and fix what you broke.",
        tools: tools!["write_file", "edit_file", "run_command", "web_fetch", "todo_write"],
    },
    RoleDef {
        id: "lead",
        name: "Lead",
        color: "#B07CFF",
        summary: "Plans the work and delegates it to other sessions",
        prompt: "You are a tech lead. Break the request into self-contained pieces, hand each one to the right role with the delegate tool (an engineer to build, a tester to verify, a reviewer to review, a researcher to find out, a writer for docs), and check what comes back. Write each delegated task so it stands on its own: the goal, the files involved, what done looks like. You don't change files yourself. Finish with a summary of what was done and what is left.",
        tools: tools!["web_fetch", "todo_write", "delegate"],
    },
    RoleDef {
        id: "reviewer",
        name: "Reviewer",
        color: "#F2B33D",
        summary: "Reviews code and reports problems, changes nothing",
        prompt: "You are a code reviewer. Read the code in question and its context, then report real problems: bugs, wrong behavior, security issues, missing tests, code that will be hard to maintain. For each one, give the file and line, what goes wrong and a fix. Skip style nits. You don't change files; you may run the tests or a linter to back up a finding.",
        tools: tools!["run_command", "todo_write"],
    },
    RoleDef {
        id: "researcher",
        name: "Researcher",
        color: "#2EC4B6",
        summary: "Finds out: reads the code and the web, answers",
        prompt: "You are a researcher. Answer the question with evidence: read the code, search it, fetch documentation from the web when needed. Say where each fact comes from (a file and line, or a URL) and what you could not confirm. You don't change files.",
        tools: tools!["web_fetch", "todo_write"],
    },
    RoleDef {
        id: "tester",
        name: "Tester",
        color: "#3DDC84",
        summary: "Writes and runs tests, reports what fails",
        prompt: "You are a test engineer. Find how the project runs its tests, run them, and write the tests that are missing for the behavior in question. When something fails, find out whether the test or the code is wrong and say so; fix tests, not product code, unless asked. Report what passes, what fails and why.",
        tools: tools!["write_file", "edit_file", "run_command", "todo_write"],
    },
    RoleDef {
        id: "writer",
        name: "Writer",
        color: "#FF7AA2",
        summary: "Writes docs, READMEs, changelogs",
        prompt: "You are a technical writer. Read the code and the existing docs, then write or update documentation that is accurate, short and easy to follow, in the project's existing tone and format. Check every command and name you mention against the code.",
        tools: tools!["write_file", "edit_file", "web_fetch", "todo_write"],
    },
    RoleDef {
        id: "custom",
        name: "Custom",
        color: "#C9CDD3",
        summary: "Your own role, every tool",
        prompt: "You are a capable assistant working on the user's computer.",
        tools: tools!["write_file", "edit_file", "run_command", "web_fetch", "todo_write", "delegate"],
    },
];

pub fn role(id: &str) -> Option<&'static RoleDef> {
    ROLES.iter().find(|r| r.id == id)
}

/// The tools a session gets: its role's, without delegation for a session
/// that was itself delegated (one level deep).
pub fn tools_for(role: &RoleDef, delegated: bool) -> Vec<&'static str> {
    role.tools.iter().copied().filter(|t| !(delegated && *t == "delegate")).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub color: &'static str,
    pub summary: &'static str,
    pub prompt: &'static str,
    pub tools: Vec<&'static str>,
}

pub fn infos() -> Vec<RoleInfo> {
    ROLES
        .iter()
        .map(|r| RoleInfo { id: r.id, name: r.name, color: r.color, summary: r.summary, prompt: r.prompt, tools: r.tools.to_vec() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{def, Access};

    #[test]
    fn roles_only_name_real_tools_and_readers_cannot_write() {
        for r in ROLES {
            for t in r.tools {
                assert!(def(t).is_some(), "{} has unknown tool {t}", r.id);
            }
        }
        for id in ["reviewer", "researcher", "lead"] {
            let r = role(id).unwrap();
            assert!(r.tools.iter().all(|t| def(t).unwrap().access != Access::Write), "{id}");
        }
        // The delegate tool's own list of roles must exist.
        for id in ["engineer", "reviewer", "researcher", "tester", "writer"] {
            assert!(role(id).is_some());
        }
    }

    #[test]
    fn a_delegated_session_cannot_delegate_again() {
        let lead = role("lead").unwrap();
        assert!(tools_for(lead, false).contains(&"delegate"));
        assert!(!tools_for(lead, true).contains(&"delegate"));
    }
}
