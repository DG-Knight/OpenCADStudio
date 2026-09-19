"""Thin Python view over OCS host API v7 entity transactions and coverage."""

class _Entity:
    def __init__(self, document, handle):
        object.__setattr__(self, "_document", document)
        object.__setattr__(self, "handle", handle)

    def _data(self):
        data = ocs.entity_descriptor(self.handle)
        if data is None:
            raise KeyError("entity %s is absent or outside the Python schema" % self.handle)
        return data

    def __getattr__(self, name):
        data = self._data()
        if name not in data:
            raise AttributeError("property %s is not in the Python entity schema" % name)
        pending = self._document._pending or {}
        value = pending.get(self.handle, {}).get(name, data[name])
        if isinstance(value, dict) and all(axis in value for axis in ("x", "y")):
            return tuple(value[axis] for axis in ("x", "y", "z") if axis in value)
        return value

    @property
    def coverage(self):
        return ocs.entity_coverage(self._data()["kind"])

    @property
    def snapshot(self):
        """Detached copy of all serialized CAD fields for this entity."""
        data = ocs.entity_snapshot(self.handle)
        if data is None:
            raise KeyError("entity %s is absent" % self.handle)
        return data[self._data()["kind"]]

    def __setattr__(self, name, value):
        if self._document._pending is None:
            raise RuntimeError("entity writes require doc.transaction(...)")
        data = self._data()
        can_edit = name in self.coverage["editable"]
        if not can_edit or name not in data or name in ("kind", "handle", "_editable"):
            raise AttributeError("property %s is not editable through this schema" % name)
        old = data[name]
        if isinstance(old, dict) and all(axis in old for axis in ("x", "y")):
            axes = tuple(axis for axis in ("x", "y", "z") if axis in old)
            if not isinstance(value, (tuple, list)) or len(value) != len(axes):
                raise ValueError("%s expects %s coordinates" % (name, len(axes)))
            value = dict(zip(axes, value))
        self._document._pending.setdefault(self.handle, {})[name] = value


class _Entities:
    def __init__(self, document):
        self._document = document

    def __getitem__(self, handle):
        entity = _Entity(self._document, int(handle))
        entity._data()
        return entity

    def __iter__(self):
        for handle in ocs.entity_handles():
            yield self[handle]

    def __len__(self):
        return len(ocs.entity_handles())


class _Transaction:
    def __init__(self, document, label):
        self.document, self.label = document, label

    def __enter__(self):
        if self.document._pending is not None:
            raise RuntimeError("nested document transactions are unsupported")
        self.document._pending = {}
        return self.document

    def __exit__(self, exc_type, exc, tb):
        updates = self.document._pending
        self.document._pending = None
        if exc_type is None and updates:
            ocs.update_many(self.label, [dict(handle=handle, **patch) for handle, patch in updates.items()])
        return False


class _Document:
    def __init__(self):
        self._pending = None
        self.entities = _Entities(self)

    def transaction(self, label):
        return _Transaction(self, label)

    def create_entity(self, kind, **properties):
        """Create one mapped entity and return its live document descriptor."""
        if "kind" in properties or "handle" in properties:
            raise ValueError("kind and handle are managed by create_entity")
        handle = ocs.add(dict(kind=kind, **properties))
        return self.entities[handle]

    def delete_entity(self, entity):
        """Delete an entity by descriptor or handle."""
        handle = entity.handle if isinstance(entity, _Entity) else int(entity)
        ocs.remove_entity(handle)

    def coverage(self, kind=None):
        if kind is not None:
            return ocs.entity_coverage(kind)
        return {name: ocs.entity_coverage(name) for name in ocs.entity_kinds()}

    def poll_events(self):
        return ocs.poll_events(ocs.tab_id())

    def request_point(self, prompt="Pick a point"):
        return ocs.request_input(prompt, False)

    def request_entity(self, prompt="Pick an entity"):
        return ocs.request_input(prompt, True)

    def poll_input(self, token):
        return ocs.poll_input(token)

    @property
    def selection(self):
        return [self.entities[handle] for handle in ocs.selection()]

    @selection.setter
    def selection(self, entities):
        ocs.select([entity.handle if isinstance(entity, _Entity) else int(entity) for entity in entities])


ocs.active_document = _Document()
