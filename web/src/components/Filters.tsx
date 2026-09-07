import { Icon } from './Icon';

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
    <div className="filters" role="search" aria-label="项目筛选">
      <label className="search-control">
        <span className="sr-only">筛选项目</span>
        <Icon name="search" />
        <input
          type="search"
          placeholder="筛选项目"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
      </label>
      <span className="filter-count" role="status">
        {count} 个项目
      </span>
      <label className="select-control">
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
    </div>
  );
}
