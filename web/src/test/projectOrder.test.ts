import { expect, it } from 'vitest';
import { sortProjects } from '../projectOrder';
import { project } from './fixtures';

it('项目按显式生命周期优先和名称稳定排序，不改变 API 数组', () => {
  const make = (name: string, value: string | null) => ({
    ...project,
    id: name,
    name,
    root: name,
    lifecycle: { value, source: null },
  });
  const projects = [
    make('灵感盒', null),
    make('山野笔记', '静态资料'),
    make('纸页', '活跃'),
    make('Studiolo', '活跃'),
    make('维护项目', '已交付维护'),
    make('暂停项目', '暂停'),
    make('候选项目', '候选'),
    make('Archive', '未建档'),
  ];
  const original = [...projects];
  expect(sortProjects(projects).map((item) => item.name)).toEqual([
    'Studiolo',
    '纸页',
    '暂停项目',
    '维护项目',
    '山野笔记',
    '候选项目',
    'Archive',
    '灵感盒',
  ]);
  expect(projects).toEqual(original);
});
