import { Icon } from './Icon';
import '../styles/projects.css';

/** 项目筛选：描边搜索框、原生 select 的生命周期，右侧等宽计数（规范第 9 节）。 */
export function Filters({
  query,
  lifecycle,
  lifecycles,
  onQueryChange,
  onLifecycleChange,
  count,
}: {
  query: string;
  lifecycle: string;
  lifecycles: string[];
  onQueryChange: (value: string) => void;
  onLifecycleChange: (value: string) => void;
  count: number;
}) {
  return (
    <div className="project-filters" role="search" aria-label="项目筛选">
      <label className="project-field project-field--search">
        <span className="sr-only">筛选项目</span>
        <Icon name="search" />
        <input
          type="search"
          placeholder="筛选项目"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
      </label>
      <label className="project-field project-field--select">
        <span className="sr-only">生命周期</span>
        <select
          value={lifecycle}
          onChange={(event) => onLifecycleChange(event.target.value)}
        >
          <option value="">全部状态</option>
          {lifecycles.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
          <option value="__unknown">未知</option>
        </select>
        <Icon name="chevron" />
      </label>
      <span className="project-filters__count" role="status">
        {count} 个项目
      </span>
    </div>
  );
}
