import { expect, test } from 'bun:test';
import { Emitter, type Event } from './events';

const status = (i: number): Event => ({ type: 'status', conn: 'c', status: 'connecting', error: null, retryIn: i });

function recorder() {
  const sizes: number[] = [];
  return { sizes, emitter: new Emitter((evs) => sizes.push(evs.length)) };
}

test('a_burst_crosses_as_one_batch', async () => {
  const { emitter, sizes } = recorder();
  for (let i = 0; i < 100; i++) emitter.emit(status(i));
  await Bun.sleep(60);
  expect(sizes).toEqual([100]);
});

test('batches_are_capped', async () => {
  const { emitter, sizes } = recorder();
  for (let i = 0; i < 2500; i++) emitter.emit(status(i));
  await Bun.sleep(80);
  expect(sizes).toEqual([2000, 500]);
});

test('events keep their order across batches', async () => {
  const seen: number[] = [];
  const emitter = new Emitter((evs) => seen.push(...evs.map((e) => (e.type === 'status' ? e.retryIn! : -1))));
  for (let i = 0; i < 2100; i++) emitter.emit(status(i));
  await Bun.sleep(80);
  expect(seen).toEqual(Array.from({ length: 2100 }, (_, i) => i));
});

test('flush delivers what is queued now', () => {
  const { emitter, sizes } = recorder();
  emitter.emit(status(1));
  emitter.emit(status(2));
  emitter.flush();
  expect(sizes).toEqual([2]);
});

test('serializes_as_the_frontend_expects', () => {
  expect(JSON.parse(JSON.stringify({ type: 'status', conn: 'c', status: 'disconnected', error: 'boom', retryIn: 4 }))).toEqual({
    type: 'status',
    conn: 'c',
    status: 'disconnected',
    error: 'boom',
    retryIn: 4,
  });
});
