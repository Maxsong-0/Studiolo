//! 只读取本代文档，不读取源码、清单、文件系统或运行平面。

use std::collections::BTreeMap;

use studiolo_core::VaultPath;

use crate::{DocumentView, ProjectArchitecture, architecture_parse, markdown};

pub(crate) fn derive(
    documents: &BTreeMap<VaultPath, DocumentView>,
    project: &str,
) -> ProjectArchitecture {
    let mut result = ProjectArchitecture {
        status: "unavailable".to_owned(),
        source: None,
        nodes: Vec::new(),
        relations: Vec::new(),
        flows: Vec::new(),
        diagnostics: Vec::new(),
    };
    let path = format!("{project}/01-框架/项目运作.md");
    let document = VaultPath::parse(&path)
        .ok()
        .and_then(|path| documents.get(&path));
    let Some(document) = document else {
        result
            .diagnostics
            .push("缺少可选的 01-框架/项目运作.md，设计声明不可用".to_owned());
        return result;
    };
    result.source = Some(markdown::location(&path, 1));
    match architecture_parse::parse(&path, &document.content) {
        Ok((nodes, relations, flows)) => {
            result.status = "ready".to_owned();
            result.nodes = nodes;
            result.relations = relations;
            result.flows = flows;
        }
        Err(reason) => {
            result.status = "invalid".to_owned();
            result.diagnostics.push(reason);
        }
    }
    result
}
