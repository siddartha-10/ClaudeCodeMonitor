import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export function useAvailableTaskLists() {
  const [lists, setLists] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<string[]>('task_lists_available')
      .then(setLists)
      .finally(() => setLoading(false));
  }, []);

  return { lists, loading, refresh: () => invoke<string[]>('task_lists_available').then(setLists) };
}
