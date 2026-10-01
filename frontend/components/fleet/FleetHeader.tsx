import { useSpacetimeDB } from 'spacetimedb/react';
import { cn } from '@/lib/utils';

export function FleetHeader() {
  const { isActive: connected } = useSpacetimeDB();
  return (
    <header className="flex items-center justify-between">
      <div>
        <h1 className="text-xl font-medium">Asimov fleet</h1>
        <p className="text-sm text-muted-foreground">VDA 5050 AGV state from SpacetimeDB</p>
      </div>
      <span className="inline-flex items-center gap-2 text-sm text-muted-foreground">
        <span className={cn('size-2 rounded-full', connected ? 'bg-online' : 'bg-destructive')} />
        {connected ? 'Live' : 'Disconnected'}
      </span>
    </header>
  );
}
