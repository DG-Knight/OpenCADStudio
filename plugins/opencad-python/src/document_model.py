"""Thin Python view over OCS host API v7 entity transactions and coverage."""

def _coordinates(value, width):
    return dict(zip("xyz"[:width], (float(axis) for axis in value)))


def _coerce_points(kind, properties):
    """Let create_entity take (x, y, z) tuples for point properties, as
    transaction edits already do, by using the property types in the schema."""
    schema = ocs.entity_coverage(kind)
    types = {row["name"]: row for row in schema.get("properties", [])} if schema else {}
    result = {}
    for name, value in properties.items():
        row = types.get(name)
        if row is not None and row["type"] in ("Vector2", "Vector3"):
            width = 2 if row["type"] == "Vector2" else 3
            if row["sequence"]:
                value = [_coordinates(item, width) if isinstance(item, (tuple, list)) else item for item in value]
            elif isinstance(value, (tuple, list)):
                value = _coordinates(value, width)
        result[name] = value
    return result


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


class _Solids:
    """Kernel-backed solid creation and rigid moves.

    The host builds and edits the geometry; a script never sees the ACIS
    payload. Every call is its own operation in the script's undo group and
    fails, leaving the drawing unchanged, when the kernel cannot do it.
    """

    def __init__(self, document):
        self._document = document

    def _create(self, primitive, values, layer):
        handle = ocs.solid_create(primitive, [float(v) for v in values], layer)
        return self._document.entities[handle]

    def box(self, center=(0, 0, 0), size=(1, 1, 1), layer=None):
        return self._create("box", tuple(center) + tuple(size), layer)

    def wedge(self, origin=(0, 0, 0), size=(1, 1, 1), layer=None):
        return self._create("wedge", tuple(origin) + tuple(size), layer)

    def cylinder(self, center=(0, 0, 0), radius=1, height=1, layer=None):
        return self._create("cylinder", tuple(center) + (radius, height), layer)

    def sphere(self, center=(0, 0, 0), radius=1, layer=None):
        return self._create("sphere", tuple(center) + (radius,), layer)

    def torus(self, center=(0, 0, 0), major=2, minor=0.5, layer=None):
        return self._create("torus", tuple(center) + (major, minor), layer)

    def pyramid(self, center=(0, 0, 0), radius=1, height=1, sides=4, layer=None):
        return self._create("pyramid", tuple(center) + (radius, height, sides), layer)

    def region(self, profile, layer=None, delete_source=False):
        """Make a planar region from one closed planar profile (a circle,
        ellipse, closed polyline or spline). Keeps the profile unless
        `delete_source` is true, and uses the profile's layer by default."""
        handle = profile.handle if isinstance(profile, _Entity) else int(profile)
        return self._document.entities[ocs.solid_region(handle, layer, bool(delete_source))]

    def _boolean(self, operation, first, second, layer, keep_operands):
        a = first.handle if isinstance(first, _Entity) else int(first)
        b = second.handle if isinstance(second, _Entity) else int(second)
        return self._document.entities[ocs.solid_boolean(a, b, operation, layer, bool(keep_operands))]

    def union(self, first, second, layer=None, keep_operands=False):
        """Join two solids into a new one. The operands are consumed unless
        `keep_operands`. The kernel may refuse (the error says why) and a
        refusal changes nothing. Curved operands can take a second or two,
        and a whole script has a 30 second budget."""
        return self._boolean("union", first, second, layer, keep_operands)

    def subtract(self, first, second, layer=None, keep_operands=False):
        """Remove `second` from `first`, giving a new solid."""
        return self._boolean("subtract", first, second, layer, keep_operands)

    def intersect(self, first, second, layer=None, keep_operands=False):
        """Keep only the volume both solids share, as a new solid."""
        return self._boolean("intersect", first, second, layer, keep_operands)

    def surface(self, profile, layer=None, delete_source=False):
        """Make a plane surface from one closed planar profile."""
        handle = profile.handle if isinstance(profile, _Entity) else int(profile)
        return self._document.entities[ocs.solid_surface(handle, layer, bool(delete_source))]

    def extrude(self, profile, direction, layer=None, delete_source=False):
        """Extrude a planar profile along `direction`: a Solid3D when the
        profile is closed, a Surface when it is open."""
        handle = profile.handle if isinstance(profile, _Entity) else int(profile)
        vector = [float(v) for v in direction]
        return self._document.entities[ocs.solid_extrude(handle, vector, layer, bool(delete_source))]

    def transform(self, entity, matrix):
        """Apply a column-major 4x4 rigid transform (16 numbers) in place."""
        handle = entity.handle if isinstance(entity, _Entity) else int(entity)
        ocs.solid_transform(handle, [float(v) for v in matrix])
        return self._document.entities[handle]

    def translate(self, entity, offset):
        dx, dy, dz = (float(v) for v in offset)
        return self.transform(entity, [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, dx, dy, dz, 1])

    def rotate(self, entity, axis, angle, about=(0, 0, 0)):
        """Turn about the world axis 'x', 'y' or 'z' through `about`."""
        index = {"x": 0, "y": 1, "z": 2}[axis]
        return self.transform(entity, ocs.rotation_matrix(index, float(angle), [float(v) for v in about]))

    def mirror(self, entity, axis, about=(0, 0, 0)):
        """Reflect across the plane normal to the world axis through `about`."""
        index = {"x": 0, "y": 1, "z": 2}[axis]
        return self.transform(entity, ocs.mirror_matrix(index, [float(v) for v in about]))


class _Layers:
    """The drawing's layer table. Every change goes through the host: it is
    validated, becomes one undo step, and a refusal raises `RuntimeError`
    without changing the drawing."""

    _KEYS = ("color", "linetype", "lineweight", "off", "frozen", "locked",
             "plottable", "transparency", "description")

    def _records(self):
        return ocs.layer_records()

    def __iter__(self):
        return iter(self._records())

    def __len__(self):
        return len(self._records())

    def __contains__(self, name):
        wanted = str(name).strip().upper()
        return any(r["name"].upper() == wanted for r in self._records())

    def __getitem__(self, name):
        wanted = str(name).strip().upper()
        for record in self._records():
            if record["name"].upper() == wanted:
                return record
        raise KeyError(name)

    def get(self, name, default=None):
        try:
            return self[name]
        except KeyError:
            return default

    def names(self):
        return [r["name"] for r in self._records()]

    @property
    def current(self):
        for record in self._records():
            if record["current"]:
                return record
        return None

    @current.setter
    def current(self, name):
        self.set_current(name)

    def _options(self, properties):
        unknown = [k for k in properties if k not in self._KEYS]
        if unknown:
            raise TypeError("unknown layer propert%s: %s" % ("y" if len(unknown) == 1 else "ies", ", ".join(sorted(unknown))))
        return {k: v for k, v in properties.items() if v is not None}

    def create(self, name, **properties):
        """Create a layer and return its record. `color` is an ACI index
        (1-255), an (r, g, b) tuple or a Color dict; `lineweight` is in
        1/100 mm (-1 ByLayer, -2 ByBlock, -3 Default); `transparency` is a
        percentage 0-90."""
        ocs.layer_operation("create", str(name), self._options(properties))
        return self[name]

    def modify(self, name, **properties):
        """Change only the properties given and return the updated record."""
        ocs.layer_operation("modify", str(name), self._options(properties))
        return self[name]

    def rename(self, name, new_name):
        ocs.layer_operation("rename", str(name), {"to": str(new_name)})
        return self[new_name]

    def delete(self, name, erase_objects=False):
        """Delete a layer. A layer that still holds objects is refused unless
        `erase_objects=True`. Layer 0, Defpoints and the current layer stay."""
        ocs.layer_operation("delete", str(name), {"erase_objects": bool(erase_objects)})

    def set_current(self, name):
        ocs.layer_operation("set_current", str(name), None)
        return self[name]


class _Styles:
    """Common lookup, rename, delete and current-style handling for the text
    and dimension style tables. Every change is validated by the host, is one
    undo step (making a style current is a setting), and a refusal raises
    `RuntimeError` without changing the drawing. `Standard` is never renamed
    or deleted, and a style that is current or in use is never deleted."""

    _KIND = None
    _RECORDS = None

    def _records(self):
        return getattr(ocs, self._RECORDS)()

    def __iter__(self):
        return iter(self._records())

    def __len__(self):
        return len(self._records())

    def __contains__(self, name):
        wanted = str(name).strip().upper()
        return any(r["name"].upper() == wanted for r in self._records())

    def __getitem__(self, name):
        wanted = str(name).strip().upper()
        for record in self._records():
            if record["name"].upper() == wanted:
                return record
        raise KeyError(name)

    def get(self, name, default=None):
        try:
            return self[name]
        except KeyError:
            return default

    def names(self):
        return [r["name"] for r in self._records()]

    @property
    def current(self):
        for record in self._records():
            if record["current"]:
                return record
        return None

    @current.setter
    def current(self, name):
        self.set_current(name)

    def _run(self, op, name, options=None):
        ocs.style_operation(self._KIND, op, str(name), options)

    def rename(self, name, new_name):
        self._run("rename", name, {"to": str(new_name)})
        return self[new_name]

    def delete(self, name):
        self._run("delete", name)

    def set_current(self, name):
        self._run("set_current", name)
        return self[name]


class _TextStyles(_Styles):
    """`ocs.active_document.text_styles`. Properties: `height` (0 = each text
    chooses), `width_factor`, `oblique` (degrees, within 85), `font` (SHX file),
    `big_font`, `backward`, `upside_down`, `vertical`, `annotative`. Give a
    TrueType face as a font file name such as `font="arial.ttf"`: the codec does
    not persist a separate TrueType family name, so it is not exposed."""

    _KIND = "text"
    _RECORDS = "text_style_records"
    _KEYS = ("height", "width_factor", "oblique", "font", "big_font",
             "backward", "upside_down", "vertical", "annotative")

    def _options(self, properties):
        unknown = [k for k in properties if k not in self._KEYS]
        if unknown:
            raise TypeError("unknown text style propert%s: %s" % ("y" if len(unknown) == 1 else "ies", ", ".join(sorted(unknown))))
        return {k: v for k, v in properties.items() if v is not None}

    def create(self, name, **properties):
        self._run("create", name, self._options(properties))
        return self[name]

    def modify(self, name, **properties):
        self._run("modify", name, self._options(properties))
        return self[name]


class _DimStyles(_Styles):
    """`ocs.active_document.dim_styles`. Records and properties use the DimStyle
    field names (`dimscale`, `dimtxt`, `dimasz`, `dimtxsty` ...); handles and
    xref fields are host-managed and refused. `create(..., copy_from="Other")`
    starts from an existing style."""

    _KIND = "dim"
    _RECORDS = "dim_style_records"

    def _options(self, properties):
        return {k: v for k, v in properties.items() if v is not None or k == "dimclrd_true_color"}

    def create(self, name, copy_from=None, **properties):
        options = self._options(properties)
        if copy_from is not None:
            options["copy_from"] = str(copy_from)
        self._run("create", name, options)
        return self[name]

    def modify(self, name, **properties):
        self._run("modify", name, self._options(properties))
        return self[name]


class _Blocks:
    """`ocs.active_document.blocks`: user block definitions (layout and
    anonymous blocks are not listed). Every change is validated by the host, is
    one undo step, and a refusal raises `RuntimeError` without changing the
    drawing. Place a block with `create_entity("Insert", block_name=...)`."""

    def _records(self):
        return ocs.block_records()

    def __iter__(self):
        return iter(self._records())

    def __len__(self):
        return len(self._records())

    def __contains__(self, name):
        wanted = str(name).strip().upper()
        return any(r["name"].upper() == wanted for r in self._records())

    def __getitem__(self, name):
        wanted = str(name).strip().upper()
        for record in self._records():
            if record["name"].upper() == wanted:
                return record
        raise KeyError(name)

    def get(self, name, default=None):
        try:
            return self[name]
        except KeyError:
            return default

    def names(self):
        return [r["name"] for r in self._records()]

    def create(self, name, entities, base_point=(0, 0, 0), erase_originals=False, description=None):
        """Define a block from existing model/paper-space entities (descriptors
        or handles). They are copied into the definition shifted by
        `-base_point`; `erase_originals=True` removes them from the drawing, as
        the BLOCK command does. No insert is placed."""
        handles = [e.handle if isinstance(e, _Entity) else int(e) for e in entities]
        options = {"entities": handles, "base_point": [float(v) for v in base_point],
                   "erase_originals": bool(erase_originals)}
        if description is not None:
            options["description"] = str(description)
        ocs.block_operation("create", str(name), options)
        return self[name]

    def modify(self, name, description=None, explodable=None, scale_uniformly=None):
        options = {k: v for k, v in (("description", description), ("explodable", explodable),
                                     ("scale_uniformly", scale_uniformly)) if v is not None}
        ocs.block_operation("modify", str(name), options)
        return self[name]

    def rename(self, name, new_name):
        """Rename a block; every insert of it follows."""
        ocs.block_operation("rename", str(name), {"to": str(new_name)})
        return self[new_name]

    def delete(self, name):
        """Delete a block that no insert, style or leader still uses."""
        ocs.block_operation("delete", str(name), None)


class _Document:
    def __init__(self):
        self._pending = None
        self.layers = _Layers()
        self.text_styles = _TextStyles()
        self.dim_styles = _DimStyles()
        self.blocks = _Blocks()
        self.entities = _Entities(self)
        self.solids = _Solids(self)

    def transaction(self, label):
        return _Transaction(self, label)

    def create_entity(self, kind, block=None, **properties):
        """Create one mapped entity and return its live document descriptor.
        With `block="Name"` the entity is added to that block definition
        instead of the drawing (its owner is set by the host)."""
        if "kind" in properties or "handle" in properties:
            raise ValueError("kind and handle are managed by create_entity")
        entity = dict(kind=kind, **_coerce_points(kind, properties))
        if block is not None:
            handle = ocs.add_to_block(str(block), entity)
        else:
            handle = ocs.add(entity)
        return self.entities[handle]

    def delete_entity(self, entity):
        """Delete an entity by descriptor or handle."""
        handle = entity.handle if isinstance(entity, _Entity) else int(entity)
        ocs.remove_entity(handle)

    def embed_picture(self, path, origin=(0, 0, 0), width=1, layer=None):
        """Embed a picture file (PNG, JPEG, BMP or another format re-encoded as
        PNG) as an OLE frame. `origin` is the bottom-left corner and `width`
        the frame width; the height follows the picture's aspect ratio."""
        handle = ocs.embed_picture(str(path), [float(v) for v in origin], float(width), layer)
        return self.entities[handle]

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
