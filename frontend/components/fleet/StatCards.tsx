import { Bot, Radio, Siren, Warehouse, type LucideIcon } from 'lucide-react';
import { Card, CardContent } from '@/components/ui/card';
import type { FleetStatsRow } from '@/module_bindings/types';

/** Fleet-wide counts; zeros until the first `fleet_stats` row arrives. */
export function StatCards({ stats }: { stats: FleetStatsRow | undefined }) {
  const cards: { label: string; value: number; icon: LucideIcon }[] = [
    { label: 'Sites', value: Number(stats?.sites ?? 0), icon: Warehouse },
    { label: 'AGVs', value: Number(stats?.agvs ?? 0), icon: Bot },
    { label: 'Online', value: Number(stats?.online ?? 0), icon: Radio },
    { label: 'Alerts', value: Number(stats?.alerts ?? 0), icon: Siren },
  ];
  return (
    <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
      {cards.map(({ label, value, icon: Icon }) => (
        <Card key={label} className="py-4">
          <CardContent className="px-5">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Icon className="size-4" />
              {label}
            </div>
            <div className="mt-2 text-4xl font-light tabular-nums text-primary">{value}</div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
