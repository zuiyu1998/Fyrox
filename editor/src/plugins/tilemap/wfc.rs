// Copyright (c) 2019-present Dmitry Stepanov and Fyrox Engine contributors.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use fyrox::{
    asset::{
        untyped::{ResourceKind, UntypedResource},
        Resource, ResourceData,
    },
    core::swap_hash_map_entry,
    fxhash::FxHashMap,
    gui::{
        button::ButtonMessage,
        check_box::{CheckBoxBuilder, CheckBoxMessage},
        formatted_text::WrapMode,
        numeric::{NumericUpDownBuilder, NumericUpDownMessage},
        stack_panel::StackPanelBuilder,
    },
    rand::thread_rng,
    scene::tilemap::{
        brush::TileMapBrushResource,
        tileset::{
            NamableValue, TileSetPropertyF32, TileSetPropertyId, TileSetPropertyNine,
            TileSetPropertyType, TileSetPropertyValueElement,
        },
        MacroTilesUpdate, TileSetWfcConstraint, TileSetWfcPropagator, TileTerrainId,
    },
};

use crate::{
    command::{Command, CommandContext, CommandTrait},
    send_sync_message,
};

use super::*;

const DEFAULT_MAX_ATTEMPTS: u32 = 10;
const DEFAULT_CONSTRAIN_EDGES: bool = true;

const PATTERN_PROP_DESC: &str = concat!("Choose a nine-slice property from the tile set. ",
    "This property will provide the pattern that the autotiler uses to know whether two tiles match along each edge. ");

const FREQUENCY_PROP_DESC: &str = concat!("Choose a float property from the tile set. ",
    "This property will provide the frequency that the autotiler uses to know know often to choose a tile when there is more than one ",
    "tile with the same pattern.");

#[derive(Default)]
pub struct WfcMacro {
    pattern_list: MacroPropertyField,
    frequency_list: MacroPropertyField,
    edges_toggle: Handle<UiNode>,
    attempts_field: Handle<UiNode>,
    terrain_list: Vec<TerrainWidgets>,
    value_field: MacroPropertyValueField,
    add_button: Handle<UiNode>,
    terrain_stack: Handle<UiNode>,
    current_terrain: TileTerrainId,
    constraint: TileSetWfcConstraint,
    propagator: TileSetWfcPropagator,
}

#[derive(Debug, Clone, Visit, Reflect, TypeUuidProvider)]
#[type_uuid(id = "24f9947e-f58b-4623-ad14-cb21cd09297e")]
pub(super) struct WfcInstance {
    frequency_property: Option<TileSetPropertyF32>,
    pattern_property: Option<TileSetPropertyNine>,
    #[reflect(hidden)]
    terrain_freq: FxHashMap<TileTerrainId, f32>,
    max_attempts: u32,
    constrain_edges: bool,
    #[reflect(hidden)]
    cells: FxHashSet<TileDefinitionHandle>,
}

impl Default for WfcInstance {
    fn default() -> Self {
        Self {
            frequency_property: None,
            pattern_property: None,
            terrain_freq: FxHashMap::default(),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            constrain_edges: DEFAULT_CONSTRAIN_EDGES,
            cells: FxHashSet::default(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct TerrainWidgets {
    terrain: TileTerrainId,
    color: Color,
    name: String,
    frequency_field: Handle<UiNode>,
    delete_button: Handle<UiNode>,
}

fn terrain_list_needs_rebuild(
    terrain_freq: &[(TileTerrainId, f32)],
    layer: Option<&TileSetPropertyLayer>,
    list: &[TerrainWidgets],
) -> bool {
    let new_iter = terrain_freq.iter().map(|&(id, _)| {
        let color;
        let name;
        if let Some(layer) = layer {
            color = layer
                .value_to_color(NamableValue::I8(id))
                .unwrap_or(ELEMENT_MATCH_HIGHLIGHT_COLOR);
            name = layer.value_to_name(NamableValue::I8(id));
        } else {
            color = ELEMENT_MATCH_HIGHLIGHT_COLOR;
            name = "".into();
        }
        (id, color, name)
    });
    let old_iter = list.iter().map(|w| (w.terrain, w.color, w.name.clone()));
    !new_iter.eq(old_iter)
}

fn sync_terrain_list(
    terrain_freq: &[(TileTerrainId, f32)],
    list: &[TerrainWidgets],
    ui: &mut UserInterface,
) {
    let freq_iter = terrain_freq.iter().map(|&(_, freq)| freq);
    let handle_iter = list.iter().map(|w| w.frequency_field);
    for (handle, freq) in handle_iter.zip(freq_iter) {
        send_sync_message(
            ui,
            NumericUpDownMessage::value(handle, MessageDirection::ToWidget, freq),
        );
    }
}

fn make_terrain_list(
    terrain_freq: &[(TileTerrainId, f32)],
    layer: Option<&TileSetPropertyLayer>,
    list: &mut Vec<TerrainWidgets>,
    ctx: &mut BuildContext,
) -> Vec<Handle<UiNode>> {
    list.clear();
    let mut result = Vec::default();
    for &(terrain, frequency) in terrain_freq {
        let (handle, widgets) = make_terrain_list_element(terrain, frequency, layer, ctx);
        list.push(widgets);
        result.push(handle);
    }
    result
}

fn make_terrain_list_element(
    terrain: TileTerrainId,
    frequency: f32,
    layer: Option<&TileSetPropertyLayer>,
    ctx: &mut BuildContext,
) -> (Handle<UiNode>, TerrainWidgets) {
    let number = TextBuilder::new(WidgetBuilder::new())
        .with_text(terrain.to_string())
        .with_horizontal_text_alignment(HorizontalAlignment::Right)
        .build(ctx);
    let color;
    let name;
    if let Some(layer) = layer {
        color = layer
            .value_to_color(NamableValue::I8(terrain))
            .unwrap_or(ELEMENT_MATCH_HIGHLIGHT_COLOR);
        name = layer.value_to_name(NamableValue::I8(terrain));
    } else {
        color = ELEMENT_MATCH_HIGHLIGHT_COLOR;
        name = "".into();
    }
    let icon = BorderBuilder::new(
        WidgetBuilder::new()
            .on_column(1)
            .with_background(Brush::Solid(color).into()),
    )
    .build(ctx);
    let name_text = TextBuilder::new(WidgetBuilder::new().on_column(2))
        .with_text(name.clone())
        .build(ctx);
    let frequency_field = NumericUpDownBuilder::new(
        WidgetBuilder::new()
            .on_column(3)
            .with_margin(Thickness::left_right(5.0)),
    )
    .with_value(frequency)
    .with_min_value(0.0)
    .build(ctx);
    let delete_button = ButtonBuilder::new(
        WidgetBuilder::new()
            .on_column(4)
            .with_margin(Thickness::uniform(2.0)),
    )
    .with_text("Delete")
    .build(ctx);
    let handle = GridBuilder::new(
        WidgetBuilder::new()
            .with_child(number)
            .with_child(icon)
            .with_child(name_text)
            .with_child(frequency_field)
            .with_child(delete_button)
            .with_margin(Thickness::uniform(2.0)),
    )
    .add_row(Row::auto())
    .add_column(Column::strict(50.0))
    .add_column(Column::strict(20.0))
    .add_column(Column::strict(100.0))
    .add_column(Column::stretch())
    .add_column(Column::strict(50.0))
    .build(ctx);
    let widgets = TerrainWidgets {
        terrain,
        color,
        name,
        frequency_field,
        delete_button,
    };
    (handle, widgets)
}

impl WfcInstance {
    fn sorted_terrain_list(&self) -> Vec<(TileTerrainId, f32)> {
        let mut result = Vec::default();
        result.extend(self.terrain_freq.iter().map(|(&id, &f)| (id, f)));
        result.sort_by_key(|&(id, _)| id);
        result
    }
}

impl ResourceData for WfcInstance {
    fn type_uuid(&self) -> Uuid {
        <Self as TypeUuidProvider>::type_uuid()
    }

    fn save(&mut self, _path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        Err("Saving is not supported!".to_string().into())
    }

    fn can_be_saved(&self) -> bool {
        false
    }
}

impl BrushMacro for WfcMacro {
    fn uuid(&self) -> &Uuid {
        &uuid!("2d14ef6a-6422-4b97-a9c9-ae5bcdfecd7e")
    }

    fn name(&self) -> &'static str {
        "Wave Function Collapse"
    }

    fn on_instance_ui_message(
        &mut self,
        context: &BrushMacroInstance,
        message: &UiMessage,
        editor: &mut Editor,
    ) {
        let ui = editor.engine.user_interfaces.first_mut();
        let Some(tile_set) = context.tile_set() else {
            return;
        };
        if let Some(TileSetPropertyMessage(uuid)) = message.data() {
            if message.destination() == self.pattern_list.handle() {
                editor.message_sender.do_command(SetPatternPropCommand {
                    brush: context.brush.clone(),
                    instance: context.settings().unwrap(),
                    data: uuid.map(TileSetPropertyNine),
                });
            } else if message.destination() == self.frequency_list.handle() {
                editor.message_sender.do_command(SetFrequencyPropCommand {
                    brush: context.brush.clone(),
                    instance: context.settings().unwrap(),
                    data: uuid.map(TileSetPropertyF32),
                });
            }
        } else if let Some(&CheckBoxMessage::Check(Some(checked))) = message.data() {
            if message.destination() == self.edges_toggle {
                editor.message_sender.do_command(SetConstrainEdgesCommand {
                    brush: context.brush.clone(),
                    instance: context.settings().unwrap(),
                    data: checked,
                });
            }
        } else if let Some(&NumericUpDownMessage::<u32>::Value(value)) = message.data() {
            if message.destination() == self.attempts_field {
                editor.message_sender.do_command(SetMaxAttemptsCommand {
                    brush: context.brush.clone(),
                    instance: context.settings().unwrap(),
                    data: value,
                });
            }
        } else if let Some(ButtonMessage::Click) = message.data() {
            if message.destination() == self.add_button {
                editor
                    .message_sender
                    .do_command(SetTerrainFrequencyCommand {
                        brush: context.brush.clone(),
                        instance: context.settings().unwrap(),
                        terrain_id: self.current_terrain,
                        data: Some(1.0),
                    });
            } else {
                for w in self.terrain_list.iter() {
                    if message.destination() == w.delete_button {
                        editor
                            .message_sender
                            .do_command(SetTerrainFrequencyCommand {
                                brush: context.brush.clone(),
                                instance: context.settings().unwrap(),
                                terrain_id: w.terrain,
                                data: None,
                            });
                    }
                }
            }
        } else if let Some(&TileSetPropertyValueMessage(TileSetPropertyValueElement::I8(id))) =
            message.data()
        {
            if message.destination() == self.value_field.handle() {
                self.current_terrain = id;
            }
        } else if let Some(&NumericUpDownMessage::<f32>::Value(frequency)) = message.data() {
            for w in self.terrain_list.iter() {
                if message.destination() == w.frequency_field {
                    editor
                        .message_sender
                        .do_command(SetTerrainFrequencyCommand {
                            brush: context.brush.clone(),
                            instance: context.settings().unwrap(),
                            terrain_id: w.terrain,
                            data: Some(frequency),
                        });
                }
            }
        } else {
            let tile_set = tile_set.data_ref();
            self.pattern_list.on_ui_message(&tile_set, message, ui);
            self.frequency_list.on_ui_message(&tile_set, message, ui);
            let instance = context.settings::<WfcInstance>().unwrap();
            let instance = instance.data_ref();
            let pattern_id = instance
                .pattern_property
                .as_ref()
                .map(|p| p.property_uuid());
            let terrain_layer = pattern_id.and_then(|id| tile_set.find_property(*id));
            self.value_field.on_ui_message(terrain_layer, message, ui);
        }
    }

    fn on_cell_ui_message(
        &mut self,
        _context: &MacroMessageContext,
        _message: &UiMessage,
        _editor: &mut Editor,
    ) {
    }

    fn create_instance(&self, _brush: &TileMapBrushResource) -> Option<UntypedResource> {
        Some(UntypedResource::new_ok(
            ResourceKind::Embedded,
            WfcInstance::default(),
        ))
    }

    fn can_create_cell(&self) -> bool {
        true
    }

    fn fill_cell_set(
        &self,
        context: &BrushMacroInstance,
        cell_set: &mut FxHashSet<TileDefinitionHandle>,
    ) {
        let Some(data) = context
            .settings
            .as_ref()
            .and_then(|r| r.try_cast::<WfcInstance>())
        else {
            return;
        };
        let data = data.data_ref();
        cell_set.extend(data.cells.iter());
    }

    fn create_cell(&self, context: &BrushMacroCell) -> Option<Command> {
        let instance = context.settings()?;
        let cell = context.cell?;
        Some(Command::new(SetCellCommand {
            brush: context.brush.clone(),
            cell,
            instance,
            included: true,
        }))
    }

    fn remove_cell(&self, context: &BrushMacroCell) -> Option<Command> {
        let instance = context.settings()?;
        let cell = context.cell?;
        Some(Command::new(SetCellCommand {
            brush: context.brush.clone(),
            cell,
            instance,
            included: false,
        }))
    }

    fn build_instance_editor(
        &mut self,
        context: &BrushMacroInstance,
        ctx: &mut BuildContext,
    ) -> Option<Handle<UiNode>> {
        let instance = context.settings::<WfcInstance>().unwrap();
        let instance = instance.data_ref();
        let pattern_id = instance
            .pattern_property
            .as_ref()
            .map(|p| p.property_uuid());
        let frequency_id = instance
            .frequency_property
            .as_ref()
            .map(|p| p.property_uuid());
        let tile_set = context.tile_set();
        let tile_set = tile_set.as_ref().map(|t| t.data_ref());
        let tile_set = tile_set.as_deref();
        self.pattern_list = MacroPropertyField::new(
            WidgetBuilder::new().with_margin(Thickness::uniform(5.0)),
            "Pattern Property".into(),
            TileSetPropertyType::NineSlice,
            pattern_id,
            tile_set,
            ctx,
        );
        self.frequency_list = MacroPropertyField::new(
            WidgetBuilder::new().with_margin(Thickness::uniform(5.0)),
            "Frequency Property".into(),
            TileSetPropertyType::F32,
            frequency_id,
            tile_set,
            ctx,
        );
        let pattern_prop_help_text =
            TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(5.0)))
                .with_wrap(WrapMode::Word)
                .with_text(PATTERN_PROP_DESC)
                .build(ctx);
        let freq_prop_help_text =
            TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(5.0)))
                .with_wrap(WrapMode::Word)
                .with_text(FREQUENCY_PROP_DESC)
                .build(ctx);
        let constrain_edges = instance.constrain_edges;
        let attempts = instance.max_attempts;
        self.attempts_field = NumericUpDownBuilder::new(WidgetBuilder::new().on_column(1))
            .with_value(attempts)
            .build(ctx);
        self.edges_toggle = CheckBoxBuilder::new(WidgetBuilder::new())
            .checked(Some(constrain_edges))
            .build(ctx);
        let edges_field = GridBuilder::new(
            WidgetBuilder::new()
                .with_child(
                    TextBuilder::new(WidgetBuilder::new().on_column(1))
                        .with_text("Constrain Edges")
                        .build(ctx),
                )
                .with_child(self.edges_toggle),
        )
        .add_row(Row::auto())
        .add_column(Column::strict(20.0))
        .add_column(Column::stretch())
        .build(ctx);
        let attempts_field = GridBuilder::new(
            WidgetBuilder::new()
                .with_child(
                    TextBuilder::new(WidgetBuilder::new())
                        .with_text("Max Attempts")
                        .build(ctx),
                )
                .with_child(self.attempts_field),
        )
        .add_row(Row::auto())
        .add_column(Column::strict(150.0))
        .add_column(Column::stretch())
        .build(ctx);
        let terrain_layer =
            tile_set.and_then(|tile_set| pattern_id.and_then(|id| tile_set.find_property(*id)));
        self.value_field = MacroPropertyValueField::new(
            WidgetBuilder::new(),
            "Terrain".into(),
            TileSetPropertyValueElement::I8(self.current_terrain),
            terrain_layer,
            ctx,
        );
        self.add_button = ButtonBuilder::new(
            WidgetBuilder::new()
                .on_column(1)
                .with_margin(Thickness::uniform(1.0)),
        )
        .with_text("Add")
        .build(ctx);
        let add_row_field = GridBuilder::new(
            WidgetBuilder::new()
                .with_child(self.value_field.handle())
                .with_child(self.add_button),
        )
        .add_row(Row::auto())
        .add_column(Column::stretch())
        .add_column(Column::strict(50.0))
        .build(ctx);
        self.terrain_stack =
            StackPanelBuilder::new(WidgetBuilder::new().with_children(make_terrain_list(
                &instance.sorted_terrain_list(),
                terrain_layer,
                &mut self.terrain_list,
                ctx,
            )))
            .build(ctx);
        let handle = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::uniform(5.0))
                .with_child(pattern_prop_help_text)
                .with_child(self.pattern_list.handle())
                .with_child(freq_prop_help_text)
                .with_child(self.frequency_list.handle())
                .with_child(edges_field)
                .with_child(attempts_field)
                .with_child(add_row_field)
                .with_child(self.terrain_stack),
        )
        .build(ctx);
        Some(handle)
    }

    fn build_cell_editor(
        &mut self,
        _context: &BrushMacroCell,
        _ctx: &mut BuildContext,
    ) -> Option<Handle<UiNode>> {
        None
    }

    fn sync_instance_editor(&mut self, context: &BrushMacroInstance, ui: &mut UserInterface) {
        let Some(instance) = context.settings::<WfcInstance>() else {
            return;
        };
        let instance = instance.data_ref();
        let pattern_id = instance
            .pattern_property
            .as_ref()
            .map(|p| p.property_uuid());
        let frequency_id = instance
            .frequency_property
            .as_ref()
            .map(|p| p.property_uuid());
        let tile_set = context.tile_set();
        let tile_set = tile_set.as_ref().map(|t| t.data_ref());
        let tile_set = tile_set.as_deref();
        self.pattern_list.sync(pattern_id, tile_set, ui);
        self.frequency_list.sync(frequency_id, tile_set, ui);
        send_sync_message(
            ui,
            CheckBoxMessage::checked(
                self.edges_toggle,
                MessageDirection::ToWidget,
                Some(instance.constrain_edges),
            ),
        );
        send_sync_message(
            ui,
            NumericUpDownMessage::<u32>::value(
                self.attempts_field,
                MessageDirection::ToWidget,
                instance.max_attempts,
            ),
        );
        let layer =
            tile_set.and_then(|tile_set| pattern_id.and_then(|id| tile_set.find_property(*id)));
        self.value_field.sync(
            TileSetPropertyValueElement::I8(self.current_terrain),
            layer,
            ui,
        );
        let terrain_freq = instance.sorted_terrain_list();
        if terrain_list_needs_rebuild(&terrain_freq, layer, &self.terrain_list) {
            let list = make_terrain_list(
                &terrain_freq,
                layer,
                &mut self.terrain_list,
                &mut ui.build_ctx(),
            );
            ui.send_message(WidgetMessage::replace_children(
                self.terrain_stack,
                MessageDirection::ToWidget,
                list,
            ));
        } else {
            sync_terrain_list(&terrain_freq, &self.terrain_list, ui);
        }
    }

    fn sync_cell_editors(&mut self, _context: &MacroMessageContext, _ui: &mut UserInterface) {}

    fn begin_update(&mut self, _context: &BrushMacroInstance, _tile_map: &TileMapContext) {}

    fn amend_update(
        &mut self,
        _context: &BrushMacroInstance,
        _update: &mut MacroTilesUpdate,
        _tile_map: &TileMap,
    ) {
    }

    fn create_command(
        &mut self,
        context: &BrushMacroInstance,
        update: &mut MacroTilesUpdate,
        tile_map: &TileMapContext,
    ) -> Option<Command> {
        let Some(tile_set) = tile_map.tile_set() else {
            self.constraint.clear();
            return None;
        };
        if context.tile_set().as_deref() != Some(tile_set) {
            self.constraint.clear();
            return None;
        }
        let instance = context.settings::<WfcInstance>().unwrap();
        let instance = instance.data_ref();
        let Some(pattern_property) = instance.pattern_property else {
            self.constraint.clear();
            return None;
        };
        let frequency_property = instance.frequency_property;
        Log::verify(self.constraint.fill_pattern_map(
            &tile_set.data_ref(),
            pattern_property,
            frequency_property,
            &instance.terrain_freq,
        ));
        let mut rng = thread_rng();
        for _ in 0..instance.max_attempts {
            self.propagator.fill_from(self.constraint.deref());
            for (&p, v) in update.iter() {
                if let Some(StampElement {
                    brush_cell: Some(cell),
                    ..
                }) = v
                {
                    if instance.cells.contains(cell) {
                        self.propagator.add_cell(p);
                    }
                }
            }
            if instance.constrain_edges
                && self
                    .propagator
                    .constrain_edges(
                        &tile_set.data_ref(),
                        pattern_property,
                        tile_map.tile_map(),
                        update,
                        self.constraint.deref(),
                    )
                    .is_err()
            {
                return None;
            }
            if let Ok(()) = self
                .propagator
                .observe_all(&mut rng, self.constraint.deref())
            {
                self.propagator
                    .apply_autotile_to_update(&mut rng, &self.constraint, update);
                return None;
            }
        }
        Log::err(format!(
            "WFC failed after {} attempts",
            instance.max_attempts
        ));
        self.propagator
            .apply_autotile_to_update(&mut rng, &self.constraint, update);
        None
    }
}

#[derive(Debug)]
struct SetCellCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub cell: TileDefinitionHandle,
    pub included: bool,
}

impl SetCellCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        let contains = instance.cells.contains(&self.cell);
        if contains != self.included {
            if self.included {
                _ = instance.cells.insert(self.cell);
            } else {
                _ = instance.cells.remove(&self.cell);
            }
            self.included = contains;
            self.brush.data_ref().change_flag.set();
        }
    }
}

impl CommandTrait for SetCellCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Wave Function Collapse Cell".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}

#[derive(Debug)]
struct SetPatternPropCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub data: Option<TileSetPropertyNine>,
}

impl SetPatternPropCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        std::mem::swap(&mut instance.pattern_property, &mut self.data);
        self.brush.data_ref().change_flag.set();
    }
}

impl CommandTrait for SetPatternPropCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Autotile Property".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}

#[derive(Debug)]
struct SetFrequencyPropCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub data: Option<TileSetPropertyF32>,
}

impl SetFrequencyPropCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        std::mem::swap(&mut instance.frequency_property, &mut self.data);
        self.brush.data_ref().change_flag.set();
    }
}

impl CommandTrait for SetFrequencyPropCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Autotile Property".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}

#[derive(Debug)]
struct SetTerrainFrequencyCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub terrain_id: TileTerrainId,
    pub data: Option<f32>,
}

impl SetTerrainFrequencyCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        swap_hash_map_entry(instance.terrain_freq.entry(self.terrain_id), &mut self.data);
        self.brush.data_ref().change_flag.set();
    }
}

impl CommandTrait for SetTerrainFrequencyCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Terrain Frequency".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}

#[derive(Debug)]
struct SetConstrainEdgesCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub data: bool,
}

impl SetConstrainEdgesCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        std::mem::swap(&mut instance.constrain_edges, &mut self.data);
        self.brush.data_ref().change_flag.set();
    }
}

impl CommandTrait for SetConstrainEdgesCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Constrain Edges".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}

#[derive(Debug)]
struct SetMaxAttemptsCommand {
    pub brush: TileMapBrushResource,
    pub instance: Resource<WfcInstance>,
    pub data: u32,
}

impl SetMaxAttemptsCommand {
    fn swap(&mut self) {
        let mut instance = self.instance.data_ref();
        std::mem::swap(&mut instance.max_attempts, &mut self.data);
        self.brush.data_ref().change_flag.set();
    }
}

impl CommandTrait for SetMaxAttemptsCommand {
    fn name(&mut self, _context: &dyn CommandContext) -> String {
        "Update Max Attempts".into()
    }

    fn execute(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }

    fn revert(&mut self, _context: &mut dyn CommandContext) {
        self.swap();
    }
}
