// Small status glyphs shared by the table, map legend, and detail card.
import {
  CircleAlert,
  CircleCheck,
  Wifi,
  WifiOff,
  type LucideIcon,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import type { Activity, ConnectionTag } from '@/lib/fleet';

const CONNECTION: Record<ConnectionTag | 'Unknown', { icon: LucideIcon; className: string; label: string }> = {
  Online: { icon: Wifi, className: 'text-online', label: 'Online' },
  Offline: { icon: WifiOff, className: 'text-muted-foreground', label: 'Offline' },
  Connectionbroken: { icon: WifiOff, className: 'text-destructive', label: 'Connection broken' },
  Unknown: { icon: Wifi, className: 'text-muted-foreground', label: 'No connection message yet' },
};

export function ConnectionIcon({ connection, className }: { connection: ConnectionTag | undefined; className?: string }) {
  const { icon: Icon, className: color, label } = CONNECTION[connection ?? 'Unknown'];
  return <Icon aria-label={label} className={cn('size-4 shrink-0', color, className)} />;
}

export function connectionLabel(connection: ConnectionTag | undefined): string {
  return CONNECTION[connection ?? 'Unknown'].label;
}

const ACTIVITY: Record<Activity, { icon: LucideIcon; className: string }> = {
  Offline: { icon: WifiOff, className: 'text-muted-foreground' },
  Error: { icon: CircleAlert, className: 'text-destructive' },
  Idle: { icon: CircleCheck, className: 'text-muted-foreground' },
};

export function ActivityLabel({ activity }: { activity: Activity }) {
  const { icon: Icon, className } = ACTIVITY[activity];
  return (
    <span className="inline-flex items-center gap-1.5">
      <Icon className={cn('size-4 shrink-0', className)} />
      {activity}
    </span>
  );
}
