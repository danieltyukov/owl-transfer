import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../App.js';
import { createMockBackend } from '../backend/mock.js';
import { freshEntries } from './Toast.js';

/*
 * `State.errors` is the last five, newest last. Once it is full a sixth error
 * pushes the first off the front and the length stops changing, so anything
 * that watermarks by counting goes silent exactly when the engine has the most
 * to say.
 */
describe('freshEntries', () => {
  it('finds the entries appended since the last look', () => {
    expect(freshEntries([], ['one'])).toEqual(['one']);
    expect(freshEntries(['one'], ['one', 'two'])).toEqual(['two']);
    expect(freshEntries(['one', 'two'], ['one', 'two'])).toEqual([]);
  });

  it('lines the window up by what is in it once it stops growing', () => {
    const full = ['a', 'b', 'c', 'd', 'e'];
    expect(freshEntries(full, ['b', 'c', 'd', 'e', 'f'])).toEqual(['f']);
    expect(freshEntries(full, ['d', 'e', 'f', 'g', 'h'])).toEqual(['f', 'g', 'h']);
  });

  it('treats a window that turned over completely as all new', () => {
    expect(freshEntries(['a', 'b'], ['c', 'd'])).toEqual(['c', 'd']);
  });

  it('counts a repeat as a separate thing that went wrong', () => {
    // The same failure twice is two failures, and a person should hear both.
    expect(freshEntries(['a'], ['a', 'a'])).toEqual(['a']);
  });

  it('says nothing was added when the list was cleared', () => {
    expect(freshEntries(['a', 'b'], [])).toEqual([]);
  });
});

describe('the toasts', () => {
  it('keeps showing engine errors after the buffer is full and rotating', async () => {
    const mock = createMockBackend({ state: { errors: [] } });
    render(<App backend={mock} />);
    await screen.findByRole('status');

    mock.emitState({ errors: ['one', 'two', 'three', 'four', 'five'] });
    await vi.waitFor(() => expect(screen.getAllByRole('alert')).toHaveLength(5));

    // The sixth pushes the first off the front. The length never changes again.
    mock.emitState({ errors: ['two', 'three', 'four', 'five', 'six'] });
    await vi.waitFor(() => expect(screen.getAllByRole('alert')).toHaveLength(6));
    expect(screen.getAllByRole('alert')[5]).toHaveTextContent('six');

    mock.emitState({ errors: ['three', 'four', 'five', 'six', 'seven'] });
    await vi.waitFor(() => expect(screen.getAllByRole('alert')).toHaveLength(7));
    expect(screen.getAllByRole('alert')[6]).toHaveTextContent('seven');
  });

  it('says the same thing once for one arrival, however often the state is pushed', async () => {
    const mock = createMockBackend({ state: { errors: [] } });
    render(<App backend={mock} />);
    await screen.findByRole('status');

    mock.emitState({ errors: ['the folder went away'] });
    await vi.waitFor(() => expect(screen.getAllByRole('alert')).toHaveLength(1));

    // The engine coalesces to ten pushes a second; the errors are unchanged.
    mock.emitState({ errors: ['the folder went away'] });
    mock.emitState({ errors: ['the folder went away'] });
    expect(screen.getAllByRole('alert')).toHaveLength(1);
  });
});
