import type { RustSourceCoverage } from '../rustCallSites';

export function RustFileCoverage({ file }: { file: RustSourceCoverage }) {
  return (
    <div className="rust-coverage">
      <h3>文件覆盖说明</h3>
      <dl>
        <div>
          <dt>文件字节</dt>
          <dd>{file.bytes}</dd>
        </div>
        <div>
          <dt>属性</dt>
          <dd>{file.attributes}</dd>
        </div>
        <div>
          <dt>其中条件属性</dt>
          <dd>{file.conditional_attributes}</dd>
        </div>
        <div>
          <dt>未展开宏调用</dt>
          <dd>{file.unexpanded_macros}</dd>
        </div>
        <div>
          <dt>宏定义</dt>
          <dd>{file.macro_definitions}</dd>
        </div>
        <div>
          <dt>外部模块声明</dt>
          <dd>{file.external_modules}</dd>
        </div>
      </dl>
      <p className="operation-muted">
        cfg / cfg_attr
        未求值，属性宏未执行；宏定义、属性和宏参数内部未提取调用。
        外部模块未跟随读取，运算符、Drop、await 等隐式调用未推导。
      </p>
    </div>
  );
}
