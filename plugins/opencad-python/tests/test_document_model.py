import pathlib
import types
import unittest


MODEL = pathlib.Path(__file__).resolve().parents[1] / "src" / "document_model.py"


class DocumentModelTests(unittest.TestCase):
    def setUp(self):
        self.records = {
            1: {"handle": 1, "kind": "Line", "layer": "0", "start": {"x": 1, "y": 2, "z": 3},
                "end": {"x": 4, "y": 5, "z": 6}, "_editable": True},
            2: {"handle": 2, "kind": "Hatch", "layer": "0", "_editable": False},
        }
        self.calls = []
        self.ocs = types.SimpleNamespace(
            entity_descriptor=lambda handle: self.records.get(handle),
            entity_coverage=lambda kind: {"kind": kind, "scope": "canvas",
                "editable": ["start", "end", "layer"] if kind == "Line" else ["layer"],
                "readable": ["handle", "kind", "layer"], "unmapped": []},
            entity_kinds=lambda: ["Line", "Hatch"],
            entity_snapshot=lambda handle: {self.records[handle]["kind"]: {
                "common": {"layer": self.records[handle]["layer"]},
                "pattern_name": "SOLID" if handle == 2 else None,
            }} if handle in self.records else None,
            entity_handles=lambda: list(self.records),
            update_many=lambda label, updates: self.calls.append((label, updates)),
            add=lambda entity: self.calls.append(("add", entity)) or 1,
            remove_entity=lambda handle: self.calls.append(("remove", handle)),
            selection=lambda: [1],
            select=lambda handles: self.calls.append(("selection", handles)),
            poll_events=lambda tab: [{"type": "drawing", "tab_id": tab}],
            tab_id=lambda: 7,
            request_input=lambda prompt, entity: (prompt, entity),
            poll_input=lambda token: {"status": "point", "point": [1, 2, 3]},
            layer_records=lambda: [{"name": "0", "current": True}, {"name": "Walls", "current": False}],
            layer_operation=lambda op, name, options: self.calls.append((op, name, options)) or 9,
        )
        namespace = {"ocs": self.ocs}
        exec(MODEL.read_text(), namespace)
        self.doc = self.ocs.active_document

    def test_batch_edit_and_rollback(self):
        line = self.doc.entities[1]
        self.assertEqual(line.start, (1, 2, 3))
        with self.doc.transaction("Move line"):
            line.start = (0, 0, 0)
            line.end = (10, 0, 0)
            self.assertEqual(line.end, (10, 0, 0))
        self.assertEqual(self.calls, [("Move line", [{"handle": 1,
            "start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 10, "y": 0, "z": 0}}])])
        with self.assertRaises(ValueError):
            with self.doc.transaction("Abort"):
                line.layer = "A"
                raise ValueError()
        self.assertEqual(len(self.calls), 1)

    def test_layers_lookup_and_options(self):
        layers = self.doc.layers
        self.assertIn("WALLS", layers)
        self.assertEqual(layers["walls"]["name"], "Walls")
        self.assertEqual(layers.current["name"], "0")
        self.assertEqual(layers.names(), ["0", "Walls"])
        with self.assertRaises(KeyError):
            layers["Nope"]
        layers.modify("Walls", color=3, linetype=None, off=False)
        self.assertEqual(self.calls[-1], ("modify", "Walls", {"color": 3, "off": False}))
        layers.delete("Walls", erase_objects=1)
        self.assertEqual(self.calls[-1], ("delete", "Walls", {"erase_objects": True}))
        with self.assertRaises(TypeError):
            layers.create("New", colour=1)
        self.assertEqual(len(self.calls), 2)

    def test_create_and_delete_use_document_model(self):
        line = self.doc.create_entity("Line", start={"x": 0, "y": 0, "z": 0},
            end={"x": 10, "y": 0, "z": 0})
        self.assertEqual(line.handle, 1)
        self.assertEqual(self.calls[-1], ("add", {"kind": "Line",
            "start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 10, "y": 0, "z": 0}}))
        self.doc.delete_entity(line)
        self.assertEqual(self.calls[-1], ("remove", 1))
        with self.assertRaises(ValueError):
            self.doc.create_entity("Line", handle=4)

    def test_create_entity_accepts_coordinate_tuples_for_point_properties(self):
        self.ocs.entity_coverage = lambda kind: {"kind": kind, "scope": "canvas",
            "editable": [], "readable": [], "unmapped": [], "properties": [
                {"name": "start", "type": "Vector3", "sequence": False},
                {"name": "end", "type": "Vector3", "sequence": False},
                {"name": "corners", "type": "Vector2", "sequence": True},
                {"name": "knots", "type": "f64", "sequence": True}]}
        self.doc.create_entity("Line", start=(0, 0, 0), end=(10, 5, 0),
            corners=[(1, 2), (3, 4)], knots=[0, 0, 1])
        self.assertEqual(self.calls[-1], ("add", {"kind": "Line",
            "start": {"x": 0.0, "y": 0.0, "z": 0.0}, "end": {"x": 10.0, "y": 5.0, "z": 0.0},
            "corners": [{"x": 1.0, "y": 2.0}, {"x": 3.0, "y": 4.0}],
            "knots": [0, 0, 1]}))

    def test_coverage_and_read_only_entity(self):
        self.assertEqual(len(self.doc.entities), 2)
        self.assertEqual([entity.handle for entity in self.doc.entities], [1, 2])
        self.assertIn("start", self.doc.entities[1].coverage["editable"])
        self.assertEqual(self.doc.entities[2].coverage["editable"], ["layer"])
        self.assertEqual(set(self.doc.coverage()), {"Line", "Hatch"})
        self.assertEqual(self.doc.coverage("Hatch")["editable"], ["layer"])
        snapshot = self.doc.entities[2].snapshot
        self.assertEqual(snapshot["pattern_name"], "SOLID")
        snapshot["pattern_name"] = "OTHER"
        self.assertEqual(self.doc.entities[2].snapshot["pattern_name"], "SOLID")
        with self.doc.transaction("Move hatch"):
            self.doc.entities[2].layer = "HATCHES"
        self.assertEqual(self.calls[-1], ("Move hatch", [{"handle": 2, "layer": "HATCHES"}]))
        with self.doc.transaction("Unsupported geometry"):
            with self.assertRaises(AttributeError):
                self.doc.entities[2].pattern_name = "OTHER"

    def test_rejected_host_transaction_leaves_python_model_ready(self):
        def reject(label, updates):
            raise ValueError("Line.start must contain finite coordinates")
        self.ocs.update_many = reject
        with self.assertRaisesRegex(ValueError, "finite coordinates"):
            with self.doc.transaction("Invalid"):
                self.doc.entities[1].start = (float("nan"), 0, 0)
        self.assertIsNone(self.doc._pending)
        self.assertEqual(self.doc.entities[1].start, (1, 2, 3))

    def test_readable_insert_identity_cannot_be_patched(self):
        self.records[3] = {"handle": 3, "kind": "Insert", "layer": "0",
            "block_name": "DOOR", "insert_point": {"x": 1, "y": 2, "z": 0},
            "_editable": True}
        old_coverage = self.ocs.entity_coverage
        self.ocs.entity_coverage = lambda kind: (
            {"kind": "Insert", "scope": "canvas", "editable": ["layer", "insert_point"],
                "readable": ["kind", "handle", "layer", "block_name", "insert_point"],
                "unmapped": []} if kind == "Insert" else old_coverage(kind))
        insert = self.doc.entities[3]
        self.assertEqual(insert.block_name, "DOOR")
        with self.doc.transaction("Move block"):
            with self.assertRaises(AttributeError):
                insert.block_name = "WINDOW"
            insert.insert_point = (4, 5, 0)
        self.assertEqual(self.calls[-1], ("Move block", [{"handle": 3,
            "insert_point": {"x": 4, "y": 5, "z": 0}}]))

    def test_selection_events_and_input(self):
        self.assertEqual([entity.handle for entity in self.doc.selection], [1])
        self.doc.selection = [self.doc.entities[1]]
        self.assertEqual(self.calls[-1], ("selection", [1]))
        self.assertEqual(self.doc.poll_events(), [{"type": "drawing", "tab_id": 7}])
        self.assertEqual(self.doc.request_entity("Pick"), ("Pick", True))
        self.assertEqual(self.doc.poll_input(10)["status"], "point")


if __name__ == "__main__":
    unittest.main()
