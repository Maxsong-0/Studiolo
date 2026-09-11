//! 按行校验预算与声明字段，最后统一核对引用；错误不发布任何部分图。

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ArchitectureFlow, ArchitectureNode, ArchitectureRelation, ArchitectureStep,
    architecture_scan::{self, Section},
    markdown, operations_parse,
};

type Graph = (
    Vec<ArchitectureNode>,
    Vec<ArchitectureRelation>,
    Vec<ArchitectureFlow>,
);

#[derive(Default)]
struct Parser {
    nodes: BTreeMap<String, ArchitectureNode>,
    relations: BTreeMap<(String, String, String), ArchitectureRelation>,
    flows: BTreeMap<String, ArchitectureFlow>,
}

pub(crate) fn parse(path: &str, content: &str) -> Result<Graph, String> {
    let mut parser = Parser::default();
    architecture_scan::scan(content, |section, line, cells| {
        parser.row(path, section, line, &cells)
    })?;
    parser.finish()
}

impl Parser {
    fn row(
        &mut self,
        path: &str,
        section: Section,
        line: u32,
        cells: &[String],
    ) -> Result<(), String> {
        // 扫描器已核对列数，每行在插入之前检查所有字段，不能先收集再截断。
        if cells.iter().any(|cell| cell.chars().any(char::is_control)) {
            return Err("字段不允许控制字符".to_owned());
        }
        match section {
            Section::Nodes => self.node(path, line, cells),
            Section::Relations => self.relation(path, line, cells),
            Section::Flows => self.step(path, line, cells),
        }
    }

    fn node(&mut self, path: &str, line: u32, cells: &[String]) -> Result<(), String> {
        identifier(&cells[0])?;
        field(&cells[1], 256)?;
        field(&cells[3], 1024)?;
        field(&cells[4], 1024)?;
        let kind = match cells[2].as_str() {
            "模块" => "module",
            "接口" => "interface",
            "外部" => "external",
            "数据" => "data",
            _ => return Err("节点类型不在声明枚举内".to_owned()),
        };
        let directory = if cells[3] == "-" {
            None
        } else {
            if !operations_parse::repository_path(&cells[3]) {
                return Err("目录必须是无隐藏段、控制字符和越界的规范相对 VaultPath".to_owned());
            }
            Some(cells[3].clone())
        };
        if self.nodes.contains_key(&cells[0]) {
            return Err("节点 ID 重复".to_owned());
        }
        if self.nodes.len() >= 64 {
            return Err("声明超过 64 个节点".to_owned());
        }
        self.nodes.insert(
            cells[0].clone(),
            ArchitectureNode {
                id: cells[0].clone(),
                label: cells[1].clone(),
                kind: kind.to_owned(),
                directory,
                description: cells[4].clone(),
                source: markdown::location(path, line),
            },
        );
        Ok(())
    }

    fn relation(&mut self, path: &str, line: u32, cells: &[String]) -> Result<(), String> {
        identifier(&cells[0])?;
        identifier(&cells[1])?;
        field(&cells[3], 256)?;
        let kind = match cells[2].as_str() {
            "依赖" => "dependency",
            "数据流" => "data_flow",
            _ => return Err("关系类型只允许依赖或数据流".to_owned()),
        };
        let key = (kind.to_owned(), cells[0].clone(), cells[1].clone());
        if self.relations.contains_key(&key) {
            return Err("相同类型、起点和终点的关系重复".to_owned());
        }
        if self.relations.len() >= 128 {
            return Err("声明超过 128 条关系".to_owned());
        }
        self.relations.insert(
            key,
            ArchitectureRelation {
                from: cells[0].clone(),
                to: cells[1].clone(),
                kind: kind.to_owned(),
                label: cells[3].clone(),
                source: markdown::location(path, line),
            },
        );
        Ok(())
    }

    fn step(&mut self, path: &str, line: u32, cells: &[String]) -> Result<(), String> {
        identifier(&cells[0])?;
        field(&cells[1], 256)?;
        identifier(&cells[3])?;
        field(&cells[4], 1024)?;
        let order = cells[2]
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=32).contains(value) && value.to_string() == cells[2])
            .ok_or_else(|| "步骤必须是 1 到 32 的十进制正整数".to_owned())?;
        if !self.flows.contains_key(&cells[0]) && self.flows.len() >= 8 {
            return Err("声明超过 8 个流程".to_owned());
        }
        let flow = self
            .flows
            .entry(cells[0].clone())
            .or_insert_with(|| ArchitectureFlow {
                id: cells[0].clone(),
                label: cells[1].clone(),
                steps: Vec::new(),
            });
        if flow.label != cells[1] {
            return Err("同一流程 ID 的名称不一致".to_owned());
        }
        if flow.steps.iter().any(|step| step.order == order) {
            return Err("同一流程的步骤编号重复".to_owned());
        }
        if flow.steps.len() >= 32 {
            return Err("单个流程超过 32 步".to_owned());
        }
        flow.steps.push(ArchitectureStep {
            order,
            node: cells[3].clone(),
            description: cells[4].clone(),
            source: markdown::location(path, line),
        });
        Ok(())
    }

    fn finish(mut self) -> Result<Graph, String> {
        if self.nodes.is_empty() {
            return Err("节点表至少需要一个节点".to_owned());
        }
        let ids: BTreeSet<_> = self.nodes.keys().map(String::as_str).collect();
        if self.relations.values().any(|relation| {
            !ids.contains(relation.from.as_str()) || !ids.contains(relation.to.as_str())
        }) {
            return Err("关系引用了本声明中不存在的节点".to_owned());
        }
        for flow in self.flows.values_mut() {
            flow.steps.sort_by_key(|step| step.order);
            if flow.steps.iter().enumerate().any(|(index, step)| {
                step.order as usize != index + 1 || !ids.contains(step.node.as_str())
            }) {
                return Err("流程步骤必须从 1 连续排列，且只能引用本声明中的节点".to_owned());
            }
        }
        Ok((
            self.nodes.into_values().collect(),
            self.relations.into_values().collect(),
            self.flows.into_values().collect(),
        ))
    }
}

fn identifier(value: &str) -> Result<(), String> {
    if value.len() > 48
        || !value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err("ID 必须符合 [a-z][a-z0-9_-]{0,47}".to_owned());
    }
    Ok(())
}

fn field(value: &str, bytes: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > bytes {
        return Err(format!(
            "名称或说明不能为空，且不得超过字段的 {bytes} 字节预算"
        ));
    }
    Ok(())
}
