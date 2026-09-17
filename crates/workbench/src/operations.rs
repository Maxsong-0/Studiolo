//! 从现有项目文档派生运作图首片；不引入第二个内容权威。

use std::collections::BTreeMap;

use studiolo_core::VaultPath;

use crate::{DocumentView, ProjectOperations, markdown, operations_block, operations_parse};

pub(crate) fn derive(
    documents: &BTreeMap<VaultPath, DocumentView>,
    project: &str,
) -> ProjectOperations {
    let mut result = ProjectOperations {
        project: project.to_owned(),
        declaration_status: "unavailable".to_owned(),
        declaration_source: None,
        directories: Vec::new(),
        comparison_parents: Vec::new(),
        diagnostics: Vec::new(),
    };
    let path = format!("{project}/01-框架/目录结构.md");
    let document = VaultPath::parse(&path)
        .ok()
        .and_then(|path| documents.get(&path));
    let Some(document) = document else {
        result
            .diagnostics
            .push("缺少 01-框架/目录结构.md，目录声明不可用".to_owned());
        return result;
    };
    result.declaration_source = Some(markdown::location(&path, 1));
    match operations_block::extract(&document.content) {
        Ok(Some(block)) => {
            result.declaration_source = Some(markdown::location(&path, block.line));
            match operations_parse::parse(&path, &block) {
                Ok((directories, parents)) => {
                    result.declaration_status = "ready".to_owned();
                    result.directories = directories;
                    result.comparison_parents = parents;
                }
                Err(reason) => {
                    result.declaration_status = "invalid".to_owned();
                    result.diagnostics.push(reason);
                }
            }
        }
        Ok(None) => result
            .diagnostics
            .push("缺少代码仓库二级章节下的 text 目录树，目录声明不可用".to_owned()),
        Err(reason) => {
            result.declaration_status = "invalid".to_owned();
            result.diagnostics.push(reason);
        }
    }
    result
}
