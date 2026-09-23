import { cx } from '../ui/dom';
import '../styles/collab.css';

export interface ProjectChoice {
  root: string;
  name: string;
}

export interface ProjectSelectProps {
  /** 当前项目 root；null 表示「全部项目」或尚未选择。 */
  value: string | null;
  options: readonly ProjectChoice[];
  onChange: (root: string | null) => void;
  /** 可见标签，也是控件的可访问名字，缺省「项目」。 */
  label?: string;
  /** 给了就多一个值为 null 的「全部项目」选项。 */
  allLabel?: string;
  /** 尚未选择时显示的提示项（不可再选回来）。 */
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}

/** 协作各标签共用的项目选择：直角、1.5px 墨线，等宽小标签在左。 */
export function ProjectSelect({
  value,
  options,
  onChange,
  label = '项目',
  allLabel,
  placeholder,
  disabled,
  className,
}: ProjectSelectProps) {
  const current = value ?? '';
  return (
    <label className={cx('collab-select', className)}>
      {label}{' '}
      <select
        className="collab-select__control"
        value={current}
        disabled={disabled}
        onChange={(event) =>
          onChange(event.target.value === '' ? null : event.target.value)
        }
      >
        {allLabel !== undefined ? (
          <option value="">{allLabel}</option>
        ) : placeholder !== undefined && current === '' ? (
          <option value="" disabled>
            {placeholder}
          </option>
        ) : null}
        {options.map((option) => (
          <option key={option.root} value={option.root}>
            {option.name}
          </option>
        ))}
      </select>
    </label>
  );
}
