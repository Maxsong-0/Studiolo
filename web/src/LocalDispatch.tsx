import { useCallback, useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { stamp } from './executionView';
import {
  Button,
  Cell,
  CellHeader,
  GridFrame,
  StatusMark,
  type StatusState,
} from './ui';
import './styles/localDispatch.css';

type Client = {
  id: string;
  provider: string;
  mode: string;
  project: string;
  cwd: string;
  ready: boolean;
  reason: string;
};
type Delivery = {
  id: string;
  client_id: string;
  task_id: string;
  state: keyof typeof deliveryLabels;
  created_ms: number;
  updated_ms: number;
  task_status?: string;
};
type Connection = { address: string; token: string };
const deliveryLabels = {
  pending: '等待本机',
  dispatching: '投递中',
  sent: '已通知（尚未证明执行）',
  uncertain: '投递结果不明，请核查本机任务',
  cancelled: '已取消通知',
  completed: '已核对黑板终态',
};
/** 通知状态的方块：只有「结果不明」要你去核查，标红；已核对终态为实心，其余待定或休眠。 */
const deliveryMarks: Record<keyof typeof deliveryLabels, StatusState> = {
  pending: 'pending',
  dispatching: 'active',
  sent: 'pending',
  uncertain: 'alert',
  cancelled: 'dormant',
  completed: 'ok',
};
class LocalError extends Error {
  constructor(readonly status: number) {
    super('本机连接请求失败');
  }
}
function safeError(error: unknown): string {
  if (error instanceof LocalError) {
    if (error.status === 401 || error.status === 403)
      return '配对已失效或没有权限，请断开后重新配对。';
    if (error.status === 409)
      return '当前工作项或客户端状态不允许派发，请刷新后核查。';
    if (error.status === 503)
      return '本机客户端暂不可用，请核查连接程序后重试。';
  }
  return '无法确认本机连接结果，请检查连接程序并重试。';
}
function localAddress(value: string): string | null {
  const match = /^http:\/\/127\.0\.0\.1:(\d{1,5})\/?$/.exec(value.trim());
  if (!match || Number(match[1]) < 1 || Number(match[1]) > 65535) return null;
  return value.trim().replace(/\/$/, '');
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new LocalError(502);
  return value as Record<string, unknown>;
}
function string(value: unknown): string {
  if (typeof value !== 'string') throw new LocalError(502);
  return value;
}
function clientsFrom(value: unknown, project: string): Client[] {
  const rows = object(value).clients;
  if (!Array.isArray(rows)) throw new LocalError(502);
  return rows
    .map((value): Client => {
      const row = object(value);
      if (typeof row.ready !== 'boolean') throw new LocalError(502);
      return {
        id: string(row.id),
        provider: string(row.provider),
        mode: string(row.mode),
        project: string(row.project),
        cwd: string(row.cwd),
        ready: row.ready,
        reason: string(row.reason),
      };
    })
    .filter((client) => client.project === project);
}
function deliveryFrom(value: unknown): Delivery {
  const row = object(value);
  const state = string(row.state);
  if (
    !Object.hasOwn(deliveryLabels, state) ||
    typeof row.created_ms !== 'number' ||
    !Number.isFinite(row.created_ms) ||
    typeof row.updated_ms !== 'number' ||
    !Number.isFinite(row.updated_ms)
  )
    throw new LocalError(502);
  return {
    id: string(row.id),
    client_id: string(row.client_id),
    task_id: string(row.task_id),
    state: state as Delivery['state'],
    created_ms: row.created_ms,
    updated_ms: row.updated_ms,
    task_status: row.task_status == null ? undefined : string(row.task_status),
  };
}
function deliveriesFrom(value: unknown): Delivery[] {
  const rows = object(value).deliveries;
  if (!Array.isArray(rows)) throw new LocalError(502);
  return rows.map(deliveryFrom);
}

export function LocalDispatch({ project }: { project: string }) {
  return <LocalDispatchProject key={project} project={project} />;
}

function LocalDispatchProject({ project }: { project: string }) {
  const [address, setAddress] = useState('http://127.0.0.1:4318');
  const [pairingCode, setPairingCode] = useState('');
  const [connection, setConnection] = useState<Connection | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [sending, setSending] = useState(false);
  const [clients, setClients] = useState<Client[]>([]);
  const [deliveries, setDeliveries] = useState<Delivery[]>([]);
  const [clientId, setClientId] = useState('');
  const [mode, setMode] = useState<'create' | 'existing'>('create');
  const [title, setTitle] = useState('');
  const [goal, setGoal] = useState('');
  const [criteria, setCriteria] = useState('');
  const [taskId, setTaskId] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const epoch = useRef(0);
  const controllers = useRef(new Set<AbortController>());
  const locks = useRef({ connect: false, refresh: false, send: false });
  const requests = useRef(new Map<string, string>());
  const selectedClient = clients.find((client) => client.id === clientId);

  const abortRequests = useCallback(() => {
    epoch.current += 1;
    for (const controller of controllers.current) controller.abort();
    controllers.current.clear();
    locks.current = { connect: false, refresh: false, send: false };
  }, []);
  useEffect(() => abortRequests, [abortRequests]);

  const request = useCallback(
    async (
      target: Connection,
      path: string,
      body?: unknown,
    ): Promise<unknown> => {
      const controller = new AbortController();
      controllers.current.add(controller);
      const deadline = setTimeout(() => controller.abort(), 10_000);
      try {
        const response = await fetch(`${target.address}${path}`, {
          method: body === undefined ? 'GET' : 'POST',
          headers: {
            Authorization: `Bearer ${target.token}`,
            'Content-Type': 'application/json',
          },
          credentials: 'omit',
          redirect: 'error',
          cache: 'no-store',
          signal: controller.signal,
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        });
        if (!response.ok) throw new LocalError(response.status);
        const value: unknown = await response.json();
        if (controller.signal.aborted) throw new LocalError(408);
        return value;
      } finally {
        clearTimeout(deadline);
        controllers.current.delete(controller);
      }
    },
    [],
  );
  const snapshot = useCallback(
    async (target: Connection, generation: number) => {
      const [clientResult, deliveryResult] = await Promise.all([
        request(target, '/v1/clients'),
        request(
          target,
          `/v1/deliveries?project=${encodeURIComponent(project)}`,
        ),
      ]);
      const nextClients = clientsFrom(clientResult, project);
      const nextDeliveries = deliveriesFrom(deliveryResult);
      if (generation !== epoch.current) return;
      setClients(nextClients);
      setDeliveries(nextDeliveries);
      setClientId((previous) =>
        nextClients.some((client) => client.id === previous)
          ? previous
          : (nextClients.find((client) => client.ready)?.id ??
            nextClients[0]?.id ??
            ''),
      );
    },
    [project, request],
  );
  const refresh = useCallback(async () => {
    if (!connection || locks.current.refresh) return;
    locks.current.refresh = true;
    setRefreshing(true);
    const generation = epoch.current;
    try {
      await snapshot(connection, generation);
      if (generation === epoch.current) setError('');
    } catch (error) {
      if (generation === epoch.current) {
        setError(safeError(error));
        setClients([]);
      }
    } finally {
      if (generation === epoch.current) {
        locks.current.refresh = false;
        setRefreshing(false);
      }
    }
  }, [connection, snapshot]);
  useEffect(() => {
    if (!connection) return;
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  }, [connection, refresh]);

  async function connect(event: FormEvent) {
    event.preventDefault();
    if (locks.current.connect) return;
    const parsedAddress = localAddress(address);
    if (!parsedAddress || !pairingCode.trim()) {
      setError('请输入 http://127.0.0.1:端口 格式的本机地址和配对码。');
      return;
    }
    locks.current.connect = true;
    setConnecting(true);
    setError('');
    setNotice('');
    const generation = epoch.current;
    const target = { address: parsedAddress, token: pairingCode.trim() };
    try {
      await snapshot(target, generation);
      if (generation !== epoch.current) return;
      setConnection(target);
      setPairingCode('');
    } catch (error) {
      if (generation === epoch.current) setError(safeError(error));
    } finally {
      if (generation === epoch.current) {
        locks.current.connect = false;
        setConnecting(false);
      }
    }
  }
  function disconnect() {
    abortRequests();
    setConnection(null);
    setPairingCode('');
    setClients([]);
    setDeliveries([]);
    setClientId('');
    setConnecting(false);
    setRefreshing(false);
    setSending(false);
    setError('');
    setNotice('');
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!connection || !selectedClient?.ready || locks.current.send) return;
    const acceptance = criteria
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean);
    if (
      mode === 'create' &&
      (!title.trim() || !goal.trim() || !acceptance.length)
    ) {
      setError('请填写标题、目标和至少一条验收条件。');
      return;
    }
    if (mode === 'existing' && !taskId.trim()) {
      setError('请填写已有工作项 ID。');
      return;
    }
    const path = mode === 'create' ? '/v1/tasks' : '/v1/dispatch';
    const payload =
      mode === 'create'
        ? {
            client_id: clientId,
            title: title.trim(),
            goal: goal.trim(),
            acceptance_criteria: acceptance,
          }
        : { client_id: clientId, task_id: taskId.trim() };
    const key = JSON.stringify([connection.address, project, path, payload]);
    let requestId = requests.current.get(key);
    if (!requestId) {
      requestId = crypto.randomUUID();
      requests.current.set(key, requestId);
    }
    locks.current.send = true;
    setSending(true);
    setError('');
    setNotice('');
    const generation = epoch.current;
    try {
      const result = await request(connection, path, {
        ...payload,
        request_id: requestId,
      });
      const delivery = deliveryFrom(
        mode === 'create' ? object(result).delivery : result,
      );
      if (generation !== epoch.current) return;
      requests.current.delete(key);
      setDeliveries((previous) => [
        delivery,
        ...previous.filter((row) => row.id !== delivery.id),
      ]);
      setNotice(
        `已登记通知，工作项 ${delivery.task_id}。执行结果以黑板回执为准。`,
      );
      if (mode === 'create') {
        setTitle('');
        setGoal('');
        setCriteria('');
      } else setTaskId('');
    } catch (error) {
      if (generation === epoch.current) setError(safeError(error));
    } finally {
      if (generation === epoch.current) {
        locks.current.send = false;
        setSending(false);
      }
    }
  }

  return (
    <details className="local-dispatch">
      <summary>本机 Codex / Claude Code</summary>
      <div className="local-dispatch-body">
        <p className="local-dispatch-intro">
          把工作项通知给已登记的本机客户端，由客户端读取黑板、认领并回写执行结果。
        </p>
        {!connection ? (
          <GridFrame className="local-dispatch-grid">
            <Cell as="div" className="local-dispatch-cell">
              <CellHeader title="配对" level={3} meta="未连接" />
              <form
                className="local-dispatch-form"
                onSubmit={(event) => void connect(event)}
              >
                <label>
                  本机地址
                  <input
                    value={address}
                    onChange={(event) => setAddress(event.target.value)}
                    disabled={connecting}
                    autoComplete="off"
                    spellCheck={false}
                  />
                </label>
                <label>
                  配对码
                  <input
                    type="password"
                    value={pairingCode}
                    onChange={(event) => setPairingCode(event.target.value)}
                    disabled={connecting}
                    autoComplete="off"
                  />
                </label>
                <p className="local-dispatch-hint">
                  配对码由本机连接程序提供，不是模型 API
                  key；仅在本次页面连接期间保留。仅使用客户端自己的订阅登录。
                </p>
                <div className="local-dispatch-actions">
                  <Button variant="primary" disabled={connecting} type="submit">
                    {connecting ? '正在连接…' : '连接本机'}
                  </Button>
                  {connecting && <Button onClick={disconnect}>取消连接</Button>}
                </div>
              </form>
            </Cell>
          </GridFrame>
        ) : (
          <GridFrame className="local-dispatch-grid">
            <Cell as="div" span={7} className="local-dispatch-cell">
              <CellHeader
                title="派发"
                level={3}
                meta={<span>已连接本机 · 每 5 秒核对</span>}
              />
              <div className="local-dispatch-actions">
                <Button
                  size="sm"
                  onClick={() => void refresh()}
                  disabled={refreshing}
                >
                  {refreshing ? '正在核对…' : '刷新本机状态'}
                </Button>
                <Button size="sm" onClick={disconnect}>
                  断开本机
                </Button>
              </div>
              {clients.length ? (
                <form
                  className="local-dispatch-form"
                  onSubmit={(event) => void submit(event)}
                >
                  <fieldset disabled={sending}>
                    <label className="local-dispatch-choice">
                      执行客户端
                      <select
                        value={clientId}
                        onChange={(event) => setClientId(event.target.value)}
                      >
                        {clients.map((client) => (
                          <option key={client.id} value={client.id}>
                            {client.provider === 'codex'
                              ? 'Codex'
                              : ['claude', 'claude_code'].includes(
                                    client.provider,
                                  )
                                ? 'Claude Code'
                                : client.provider}{' '}
                            · {client.id}
                            {client.ready ? '' : '（不可用）'}
                          </option>
                        ))}
                      </select>
                    </label>
                    {selectedClient && (
                      <div className="local-dispatch-client">
                        <span>会话方式：{selectedClient.mode}</span>
                        <span>工作目录：{selectedClient.cwd}</span>
                        {!selectedClient.ready && (
                          <p role="status" className="local-dispatch-blocked">
                            暂不可派发：
                            {selectedClient.reason || '客户端尚未准备好。'}
                          </p>
                        )}
                      </div>
                    )}
                    <label className="local-dispatch-choice">
                      派发内容
                      <select
                        value={mode}
                        onChange={(event) =>
                          setMode(
                            event.target.value === 'existing'
                              ? 'existing'
                              : 'create',
                          )
                        }
                      >
                        <option value="create">创建只读工作项</option>
                        <option value="existing">通知已有工作项</option>
                      </select>
                    </label>
                    {mode === 'create' ? (
                      <>
                        <p className="local-dispatch-hint">
                          新工作项仅做读取与分析；需要写入时，请使用黑板上已授权写域的工作项。
                        </p>
                        <label>
                          标题
                          <input
                            value={title}
                            onChange={(event) => setTitle(event.target.value)}
                            maxLength={200}
                            required
                          />
                        </label>
                        <label>
                          目标
                          <textarea
                            value={goal}
                            onChange={(event) => setGoal(event.target.value)}
                            rows={3}
                            required
                          />
                        </label>
                        <label>
                          验收条件（每行一条）
                          <textarea
                            value={criteria}
                            onChange={(event) =>
                              setCriteria(event.target.value)
                            }
                            rows={3}
                            required
                          />
                        </label>
                      </>
                    ) : (
                      <label>
                        已有工作项 ID
                        <input
                          value={taskId}
                          onChange={(event) => setTaskId(event.target.value)}
                          required
                          autoComplete="off"
                          spellCheck={false}
                        />
                      </label>
                    )}
                    <div className="local-dispatch-actions">
                      <Button
                        variant="primary"
                        type="submit"
                        disabled={!selectedClient?.ready || sending}
                      >
                        {sending
                          ? '正在登记…'
                          : mode === 'create'
                            ? '创建并通知本机'
                            : '通知本机'}
                      </Button>
                    </div>
                  </fieldset>
                </form>
              ) : (
                <p className="local-dispatch-hint">
                  当前项目没有可用的本机客户端，请在连接程序中登记后刷新。
                </p>
              )}
            </Cell>
            <Cell
              span={5}
              className="local-dispatch-cell"
              aria-label="本机通知记录"
            >
              <CellHeader
                title="本机通知记录"
                level={3}
                meta={`${deliveries.length} 条`}
              />
              <p className="local-dispatch-hint">
                “已通知”只表示通知已送出；认领、执行和完成状态以黑板为准。
              </p>
              {deliveries.length ? (
                <ol className="local-dispatch-deliveries">
                  {deliveries.map((delivery) => {
                    const updated = stamp(delivery.updated_ms);
                    const created = stamp(delivery.created_ms);
                    return (
                      <li key={delivery.id} className="local-dispatch-delivery">
                        {updated ? (
                          <time
                            className="local-dispatch-time"
                            dateTime={updated.iso}
                            title={`更新于 ${updated.text}${created ? `，登记于 ${created.text}` : ''}`}
                          >
                            {updated.text}
                          </time>
                        ) : (
                          <span
                            className="local-dispatch-time"
                            title="本机连接程序给出的时间超出可显示范围。"
                          >
                            —
                          </span>
                        )}
                        <StatusMark
                          state={deliveryMarks[delivery.state]}
                          className="local-dispatch-state"
                        >
                          {deliveryLabels[delivery.state]}
                        </StatusMark>
                        <span>
                          工作项 <code>{delivery.task_id}</code>
                        </span>
                        <span>客户端 {delivery.client_id}</span>
                        {delivery.task_status && (
                          <span>黑板状态：{delivery.task_status}</span>
                        )}
                      </li>
                    );
                  })}
                </ol>
              ) : (
                <p className="local-dispatch-hint">暂无本机通知。</p>
              )}
            </Cell>
          </GridFrame>
        )}
        {error && (
          <p role="alert" className="local-dispatch-error">
            {error}
          </p>
        )}
        {notice && (
          <p role="status" className="local-dispatch-notice">
            {notice}
          </p>
        )}
      </div>
    </details>
  );
}
