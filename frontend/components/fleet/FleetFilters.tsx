import { Search } from 'lucide-react';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import type { Map as MapRow } from '@/module_bindings/types';

interface FleetFiltersProps {
  query: string;
  onQueryChange: (query: string) => void;
  mapId: string | undefined;
  maps: readonly MapRow[];
  onMapChange: (mapId: string) => void;
}

/** Search box and site picker above the AGV table. */
export function FleetFilters({ query, onQueryChange, mapId, maps, onMapChange }: FleetFiltersProps) {
  return (
    <div className="flex flex-wrap gap-3">
      <div className="relative w-72">
        <Search className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query}
          onChange={e => onQueryChange(e.target.value)}
          placeholder="Search AGVs"
          className="pl-9"
        />
      </div>
      <Select value={mapId ?? ''} onValueChange={onMapChange}>
        <SelectTrigger className="w-44">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {maps.map(m => (
            <SelectItem key={m.mapId} value={m.mapId}>
              {m.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
