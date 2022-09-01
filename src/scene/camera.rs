//! Contains all methods and structures to create and manage cameras.
//!
//! Camera allows you to see world from specific point in world. Currently only
//! perspective projection is supported.
//!
//! # Multiple cameras
//!
//! Fyrox supports multiple cameras per scene, it means that you can create split
//! screen games, make picture-in-picture insertions in your main camera view and
//! any other combinations you need.
//!
//! ## Performance
//!
//! Each camera forces engine to re-render same scene one more time, which may cause
//! almost double load of your GPU.

use crate::{
    core::{
        algebra::{Matrix4, Point3, Vector2, Vector3, Vector4},
        inspect::{Inspect, PropertyInfo},
        math::{aabb::AxisAlignedBoundingBox, frustum::Frustum, ray::Ray, Rect},
        pool::Handle,
        reflect::Reflect,
        uuid::{uuid, Uuid},
        variable::TemplateVariable,
        visitor::{Visit, VisitResult, Visitor},
    },
    engine::resource_manager::ResourceManager,
    resource::texture::{Texture, TextureError, TextureKind, TexturePixelKind, TextureWrapMode},
    scene::{
        base::{Base, BaseBuilder},
        graph::{map::NodeHandleMap, Graph},
        node::{Node, NodeTrait, TypeUuidProvider, UpdateContext},
        visibility::VisibilityCache,
    },
    utils::log::Log,
};
use fyrox_resource::ResourceState;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use strum_macros::{AsRefStr, EnumString, EnumVariantNames};

/// Perspective projection make parallel lines to converge at some point. Objects will be smaller
/// with increasing distance. This the projection type "used" by human eyes, photographic lens and
/// it looks most realistic.
#[derive(Inspect, Reflect, Clone, Debug, PartialEq, Visit)]
pub struct PerspectiveProjection {
    /// Horizontal angle between look axis and a side of the viewing frustum. Larger values will
    /// increase field of view and create fish-eye effect, smaller values could be used to create
    /// "binocular" effect or scope effect.  
    #[inspect(min_value = 0.0, max_value = 3.14159, step = 0.1)]
    pub fov: f32,
    /// Location of the near clipping plane.
    #[inspect(min_value = 0.0, step = 0.1)]
    pub z_near: f32,
    /// Location of the far clipping plane.
    #[inspect(min_value = 0.0, step = 0.1)]
    pub z_far: f32,
}

impl Default for PerspectiveProjection {
    fn default() -> Self {
        Self {
            fov: 75.0f32.to_radians(),
            z_near: 0.025,
            z_far: 2048.0,
        }
    }
}

impl PerspectiveProjection {
    /// Returns perspective projection matrix.
    #[inline]
    pub fn matrix(&self, frame_size: Vector2<f32>) -> Matrix4<f32> {
        Matrix4::new_perspective(
            (frame_size.x / frame_size.y).max(10.0 * f32::EPSILON),
            self.fov,
            self.z_near,
            self.z_far,
        )
    }
}

/// Parallel projection. Object's size won't be affected by distance from the viewer, it can be
/// used for 2D games.
#[derive(Inspect, Reflect, Clone, Debug, PartialEq, Visit)]
pub struct OrthographicProjection {
    /// Location of the near clipping plane.
    #[inspect(min_value = 0.0, step = 0.1)]
    pub z_near: f32,
    /// Location of the far clipping plane.
    #[inspect(min_value = 0.0, step = 0.1)]
    pub z_far: f32,
    /// Vertical size of the "view box". Horizontal size is derived value and depends on the aspect
    /// ratio of the viewport.
    #[inspect(step = 0.1)]
    pub vertical_size: f32,
}

impl Default for OrthographicProjection {
    fn default() -> Self {
        Self {
            z_near: 0.0,
            z_far: 2048.0,
            vertical_size: 5.0,
        }
    }
}

impl OrthographicProjection {
    /// Returns orthographic projection matrix.
    #[inline]
    pub fn matrix(&self, frame_size: Vector2<f32>) -> Matrix4<f32> {
        let aspect = (frame_size.x / frame_size.y).max(10.0 * f32::EPSILON);
        let horizontal_size = aspect * self.vertical_size;

        let left = -horizontal_size;
        let top = self.vertical_size;
        let right = horizontal_size;
        let bottom = -self.vertical_size;
        Matrix4::new_orthographic(left, right, bottom, top, self.z_near, self.z_far)
    }
}

/// A method of projection. Different projection types suitable for different purposes:
///
/// 1) Perspective projection most useful for 3D games, it makes a scene to look most natural,
/// objects will look smaller with increasing distance.
/// 2) Orthographic projection most useful for 2D games, objects won't look smaller with increasing
/// distance.  
#[derive(
    Inspect, Reflect, Clone, Debug, PartialEq, Visit, AsRefStr, EnumString, EnumVariantNames,
)]
pub enum Projection {
    /// See [`PerspectiveProjection`] docs.
    Perspective(PerspectiveProjection),
    /// See [`OrthographicProjection`] docs.
    Orthographic(OrthographicProjection),
}

impl Projection {
    /// Sets the new value for the near clipping plane.
    #[inline]
    #[must_use]
    pub fn with_z_near(mut self, z_near: f32) -> Self {
        match self {
            Projection::Perspective(ref mut v) => v.z_near = z_near,
            Projection::Orthographic(ref mut v) => v.z_near = z_near,
        }
        self
    }

    /// Sets the new value for the far clipping plane.
    #[inline]
    #[must_use]
    pub fn with_z_far(mut self, z_far: f32) -> Self {
        match self {
            Projection::Perspective(ref mut v) => v.z_far = z_far,
            Projection::Orthographic(ref mut v) => v.z_far = z_far,
        }
        self
    }

    /// Sets the new value for the near clipping plane.
    #[inline]
    pub fn set_z_near(&mut self, z_near: f32) {
        match self {
            Projection::Perspective(v) => v.z_near = z_near,
            Projection::Orthographic(v) => v.z_near = z_near,
        }
    }

    /// Sets the new value for the far clipping plane.
    #[inline]
    pub fn set_z_far(&mut self, z_far: f32) {
        match self {
            Projection::Perspective(v) => v.z_far = z_far,
            Projection::Orthographic(v) => v.z_far = z_far,
        }
    }

    /// Returns near clipping plane distance.
    #[inline]
    pub fn z_near(&self) -> f32 {
        match self {
            Projection::Perspective(v) => v.z_near,
            Projection::Orthographic(v) => v.z_near,
        }
    }

    /// Returns far clipping plane distance.
    #[inline]
    pub fn z_far(&self) -> f32 {
        match self {
            Projection::Perspective(v) => v.z_far,
            Projection::Orthographic(v) => v.z_far,
        }
    }

    /// Returns projection matrix.
    #[inline]
    pub fn matrix(&self, frame_size: Vector2<f32>) -> Matrix4<f32> {
        match self {
            Projection::Perspective(v) => v.matrix(frame_size),
            Projection::Orthographic(v) => v.matrix(frame_size),
        }
    }
}

impl Default for Projection {
    fn default() -> Self {
        Self::Perspective(PerspectiveProjection::default())
    }
}

/// Exposure is a parameter that describes how many light should be collected for one
/// frame. The higher the value, the more brighter the final frame will be and vice versa.
#[derive(
    Visit, Copy, Clone, PartialEq, Debug, Inspect, Reflect, AsRefStr, EnumString, EnumVariantNames,
)]
pub enum Exposure {
    /// Automatic exposure based on the frame luminance. High luminance values will result
    /// in lower exposure levels and vice versa. This is default option.
    ///
    /// # Equation
    ///
    /// `exposure = key_value / clamp(avg_luminance, min_luminance, max_luminance)`
    Auto {
        /// A key value in the formula above. Default is 0.01556.
        #[inspect(min_value = 0.0, step = 0.1)]
        key_value: f32,
        /// A min luminance value in the formula above. Default is 0.00778.
        #[inspect(min_value = 0.0, step = 0.1)]
        min_luminance: f32,
        /// A max luminance value in the formula above. Default is 64.0.
        #[inspect(min_value = 0.0, step = 0.1)]
        max_luminance: f32,
    },

    /// Specific exposure level. To "disable" any HDR effects use [`std::f32::consts::E`] as a value.
    Manual(f32),
}

impl Default for Exposure {
    fn default() -> Self {
        Self::Auto {
            key_value: 0.01556,
            min_luminance: 0.00778,
            max_luminance: 64.0,
        }
    }
}

/// See module docs.
#[derive(Debug, Visit, Inspect, Reflect, Clone)]
pub struct Camera {
    base: Base,

    #[inspect(deref)]
    #[reflect(setter = "set_projection")]
    projection: TemplateVariable<Projection>,

    #[inspect(deref)]
    #[reflect(setter = "set_viewport")]
    viewport: TemplateVariable<Rect<f32>>,

    #[inspect(deref)]
    #[reflect(setter = "set_enabled")]
    enabled: TemplateVariable<bool>,

    #[inspect(deref)]
    #[reflect(setter = "set_skybox")]
    sky_box: TemplateVariable<Option<SkyBox>>,

    #[inspect(deref)]
    #[reflect(setter = "set_environment")]
    environment: TemplateVariable<Option<Texture>>,

    #[inspect(deref)]
    #[reflect(setter = "set_exposure")]
    exposure: TemplateVariable<Exposure>,

    #[inspect(deref)]
    #[reflect(setter = "set_color_grading_lut")]
    color_grading_lut: TemplateVariable<Option<ColorGradingLut>>,

    #[inspect(deref)]
    #[reflect(setter = "set_color_grading_enabled")]
    color_grading_enabled: TemplateVariable<bool>,

    #[visit(skip)]
    #[inspect(skip)]
    #[reflect(hidden)]
    view_matrix: Matrix4<f32>,

    #[visit(skip)]
    #[inspect(skip)]
    #[reflect(hidden)]
    projection_matrix: Matrix4<f32>,

    /// Visibility cache allows you to quickly check if object is visible from the camera or not.
    #[visit(skip)]
    #[inspect(skip)]
    #[reflect(hidden)]
    pub visibility_cache: VisibilityCache,
}

impl Deref for Camera {
    type Target = Base;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for Camera {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl Default for Camera {
    fn default() -> Self {
        CameraBuilder::new(BaseBuilder::new()).build_camera()
    }
}

impl TypeUuidProvider for Camera {
    fn type_uuid() -> Uuid {
        uuid!("198d3aca-433c-4ce1-bb25-3190699b757f")
    }
}

impl Camera {
    /// Explicitly calculates view and projection matrices. Normally, you should not call
    /// this method, it will be called automatically when new frame starts.
    #[inline]
    pub fn calculate_matrices(&mut self, frame_size: Vector2<f32>) {
        let pos = self.base.global_position();
        let look = self.base.look_vector();
        let up = self.base.up_vector();

        self.view_matrix = Matrix4::look_at_rh(&Point3::from(pos), &Point3::from(pos + look), &up);
        self.projection_matrix = self.projection.matrix(frame_size);
    }

    /// Sets new viewport in resolution-independent format. In other words
    /// each parameter of viewport defines portion of your current resolution
    /// in percents. In example viewport (0.0, 0.0, 0.5, 1.0) will force camera
    /// to use left half of your screen and (0.5, 0.0, 0.5, 1.0) - right half.
    /// Why not just use pixels directly? Because you can change resolution while
    /// your application is running and you'd be force to manually recalculate
    /// pixel values everytime when resolution changes.
    pub fn set_viewport(&mut self, mut viewport: Rect<f32>) -> Rect<f32> {
        viewport.position.x = viewport.position.x.clamp(0.0, 1.0);
        viewport.position.y = viewport.position.y.clamp(0.0, 1.0);
        viewport.size.x = viewport.size.x.clamp(0.0, 1.0);
        viewport.size.y = viewport.size.y.clamp(0.0, 1.0);
        self.viewport.set(viewport)
    }

    /// Returns current viewport.
    pub fn viewport(&self) -> Rect<f32> {
        *self.viewport
    }

    /// Calculates viewport rectangle in pixels based on internal resolution-independent
    /// viewport. It is useful when you need to get real viewport rectangle in pixels.
    ///
    /// # Notes
    ///
    /// Viewport cannot be less than 1x1 pixel in size, so the method clamps values to
    /// range `[1; infinity]`. This is strictly needed because having viewport of 0 in size
    /// will cause panics in various places. It happens because viewport size is used as
    /// divisor in math formulas, but you cannot divide by zero.
    #[inline]
    pub fn viewport_pixels(&self, frame_size: Vector2<f32>) -> Rect<i32> {
        Rect::new(
            (self.viewport.x() * frame_size.x) as i32,
            (self.viewport.y() * frame_size.y) as i32,
            ((self.viewport.w() * frame_size.x) as i32).max(1),
            ((self.viewport.h() * frame_size.y) as i32).max(1),
        )
    }

    /// Returns current view-projection matrix.
    #[inline]
    pub fn view_projection_matrix(&self) -> Matrix4<f32> {
        self.projection_matrix * self.view_matrix
    }

    /// Returns current projection matrix.
    #[inline]
    pub fn projection_matrix(&self) -> Matrix4<f32> {
        self.projection_matrix
    }

    /// Returns current view matrix.
    #[inline]
    pub fn view_matrix(&self) -> Matrix4<f32> {
        self.view_matrix
    }

    /// Returns inverse view matrix.
    #[inline]
    pub fn inv_view_matrix(&self) -> Option<Matrix4<f32>> {
        self.view_matrix.try_inverse()
    }

    /// Returns current projection mode.
    #[inline]
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// Returns current projection mode.
    #[inline]
    pub fn projection_value(&self) -> Projection {
        (*self.projection).clone()
    }

    /// Returns current projection mode as mutable reference.
    #[inline]
    pub fn projection_mut(&mut self) -> &mut Projection {
        self.projection.get_mut()
    }

    /// Sets current projection mode.
    #[inline]
    pub fn set_projection(&mut self, projection: Projection) -> Projection {
        self.projection.set(projection)
    }

    /// Returns state of camera: enabled or not.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        *self.enabled
    }

    /// Enables or disables camera. Disabled cameras will be ignored during
    /// rendering. This allows you to exclude views from specific cameras from
    /// final picture.
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        self.enabled.set(enabled)
    }

    /// Sets new skybox. Could be None if no skybox needed.
    pub fn set_skybox(&mut self, skybox: Option<SkyBox>) -> Option<SkyBox> {
        self.sky_box.set(skybox)
    }

    /// Return optional mutable reference to current skybox.
    pub fn skybox_mut(&mut self) -> Option<&mut SkyBox> {
        self.sky_box.get_mut().as_mut()
    }

    /// Return optional shared reference to current skybox.
    pub fn skybox_ref(&self) -> Option<&SkyBox> {
        self.sky_box.as_ref()
    }

    /// Replaces the skybox.
    pub fn replace_skybox(&mut self, new: Option<SkyBox>) -> Option<SkyBox> {
        std::mem::replace(self.sky_box.get_mut(), new)
    }

    /// Sets new environment.
    pub fn set_environment(&mut self, environment: Option<Texture>) -> Option<Texture> {
        self.environment.set(environment)
    }

    /// Return optional mutable reference to current environment.
    pub fn environment_mut(&mut self) -> Option<&mut Texture> {
        self.environment.get_mut().as_mut()
    }

    /// Return optional shared reference to current environment.
    pub fn environment_ref(&self) -> Option<&Texture> {
        self.environment.as_ref()
    }

    /// Return current environment map.
    pub fn environment_map(&self) -> Option<Texture> {
        (*self.environment).clone()
    }

    /// Creates picking ray from given screen coordinates.
    pub fn make_ray(&self, screen_coord: Vector2<f32>, screen_size: Vector2<f32>) -> Ray {
        let viewport = self.viewport_pixels(screen_size);
        let nx = screen_coord.x / (viewport.w() as f32) * 2.0 - 1.0;
        // Invert y here because OpenGL has origin at left bottom corner,
        // but window coordinates starts from left *upper* corner.
        let ny = (viewport.h() as f32 - screen_coord.y) / (viewport.h() as f32) * 2.0 - 1.0;
        let inv_view_proj = self
            .view_projection_matrix()
            .try_inverse()
            .unwrap_or_default();
        let near = inv_view_proj * Vector4::new(nx, ny, -1.0, 1.0);
        let far = inv_view_proj * Vector4::new(nx, ny, 1.0, 1.0);
        let begin = near.xyz().scale(1.0 / near.w);
        let end = far.xyz().scale(1.0 / far.w);
        Ray::from_two_points(begin, end)
    }

    /// Projects given world space point on screen plane.
    pub fn project(
        &self,
        world_pos: Vector3<f32>,
        screen_size: Vector2<f32>,
    ) -> Option<Vector2<f32>> {
        let viewport = self.viewport_pixels(screen_size);
        let proj = self.view_projection_matrix()
            * Vector4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
        if proj.w != 0.0 && proj.z >= 0.0 {
            let k = (1.0 / proj.w) * 0.5;
            Some(Vector2::new(
                viewport.x() as f32 + viewport.w() as f32 * (proj.x * k + 0.5),
                viewport.h() as f32
                    - (viewport.y() as f32 + viewport.h() as f32 * (proj.y * k + 0.5)),
            ))
        } else {
            None
        }
    }

    /// Sets new color grading LUT.
    pub fn set_color_grading_lut(
        &mut self,
        lut: Option<ColorGradingLut>,
    ) -> Option<ColorGradingLut> {
        self.color_grading_lut.set(lut)
    }

    /// Returns current color grading map.
    pub fn color_grading_lut(&self) -> Option<ColorGradingLut> {
        (*self.color_grading_lut).clone()
    }

    /// Returns current color grading map by ref.
    pub fn color_grading_lut_ref(&self) -> Option<&ColorGradingLut> {
        self.color_grading_lut.as_ref()
    }

    /// Enables or disables color grading.
    pub fn set_color_grading_enabled(&mut self, enable: bool) -> bool {
        self.color_grading_enabled.set(enable)
    }

    /// Whether color grading enabled or not.
    pub fn color_grading_enabled(&self) -> bool {
        *self.color_grading_enabled
    }

    /// Sets new exposure. See `Exposure` struct docs for more info.
    pub fn set_exposure(&mut self, exposure: Exposure) -> Exposure {
        self.exposure.set(exposure)
    }

    /// Returns current exposure value.
    pub fn exposure(&self) -> Exposure {
        *self.exposure
    }
}

impl NodeTrait for Camera {
    crate::impl_query_component!();

    /// Returns current **local-space** bounding box.
    #[inline]
    fn local_bounding_box(&self) -> AxisAlignedBoundingBox {
        // TODO: Maybe calculate AABB using frustum corners?
        self.base.local_bounding_box()
    }

    /// Returns current **world-space** bounding box.
    fn world_bounding_box(&self) -> AxisAlignedBoundingBox {
        self.base.world_bounding_box()
    }

    fn restore_resources(&mut self, resource_manager: ResourceManager) {
        self.base.restore_resources(resource_manager.clone());

        let mut state = resource_manager.state();
        let texture_container = &mut state.containers_mut().textures;
        texture_container.try_restore_template_resource(&mut self.environment);

        if let Some(skybox) = self.skybox_mut() {
            texture_container.try_restore_optional_resource(&mut skybox.bottom);
            texture_container.try_restore_optional_resource(&mut skybox.top);
            texture_container.try_restore_optional_resource(&mut skybox.left);
            texture_container.try_restore_optional_resource(&mut skybox.right);
            texture_container.try_restore_optional_resource(&mut skybox.front);
            texture_container.try_restore_optional_resource(&mut skybox.back);
        }
    }

    fn remap_handles(&mut self, old_new_mapping: &NodeHandleMap) {
        self.base.remap_handles(old_new_mapping);
    }

    fn id(&self) -> Uuid {
        Self::type_uuid()
    }

    fn update(&mut self, context: &mut UpdateContext) -> bool {
        self.calculate_matrices(context.frame_size);

        self.visibility_cache.clear();
        self.visibility_cache.update(
            context.nodes,
            self.global_position(),
            self.projection().z_near(),
            self.projection().z_far(),
            Some(&[&Frustum::from(self.view_projection_matrix()).unwrap_or_default()]),
        );

        self.base.update_lifetime(context.dt)
    }
}

/// All possible error that may occur during color grading look-up table creation.
#[derive(Debug, thiserror::Error)]
pub enum ColorGradingLutCreationError {
    /// There is not enough data in provided texture to build LUT.
    #[error(
        "There is not enough data in provided texture to build LUT. Required: {}, current: {}.",
        required,
        current
    )]
    NotEnoughData {
        /// Required amount of bytes.
        required: usize,
        /// Actual data size.
        current: usize,
    },

    /// Pixel format is not supported. It must be either RGB8 or RGBA8.
    #[error("Pixel format is not supported. It must be either RGB8 or RGBA8, but texture has {0:?} pixel format")]
    InvalidPixelFormat(TexturePixelKind),

    /// Texture error.
    #[error("Texture load error: {0:?}")]
    Texture(Option<Arc<TextureError>>),
}

/// Color grading look up table (LUT). Color grading is used to modify color space of the
/// rendered frame; it maps one color space to another. It is widely used effect in games,
/// you've probably noticed either "warmness" or "coldness" in colors in various scenes in
/// games - this is achieved by color grading.
///
/// See [more info in Unreal engine docs](https://docs.unrealengine.com/4.26/en-US/RenderingAndGraphics/PostProcessEffects/UsingLUTs/)
#[derive(Visit, Clone, Default, PartialEq, Debug, Inspect, Reflect, Eq)]
pub struct ColorGradingLut {
    unwrapped_lut: Option<Texture>,

    #[visit(skip)]
    #[inspect(skip)]
    #[reflect(hidden)]
    lut: Option<Texture>,
}

impl ColorGradingLut {
    /// Creates 3D look-up texture from 2D strip.
    ///
    /// # Input Texture Requirements
    ///
    /// Width: 1024px
    /// Height: 16px
    /// Pixel Format: RGB8/RGBA8
    ///
    /// # Usage
    ///
    /// Typical usage would be:
    ///
    /// ```no_run
    /// use fyrox::scene::camera::ColorGradingLut;
    /// use fyrox::engine::resource_manager::{ResourceManager};
    ///
    /// async fn create_lut(resource_manager: ResourceManager) -> ColorGradingLut {
    ///     ColorGradingLut::new(resource_manager.request_texture(
    ///         "your_lut.jpg",
    ///     ))
    ///     .await
    ///     .unwrap()
    /// }
    /// ```
    ///
    /// Then pass LUT to either CameraBuilder or to camera instance, and don't forget to enable
    /// color grading.
    pub async fn new(unwrapped_lut: Texture) -> Result<Self, ColorGradingLutCreationError> {
        match unwrapped_lut.await {
            Ok(unwrapped_lut) => {
                let data = unwrapped_lut.data_ref();

                if data.pixel_kind() != TexturePixelKind::RGBA8
                    && data.pixel_kind() != TexturePixelKind::RGB8
                {
                    return Err(ColorGradingLutCreationError::InvalidPixelFormat(
                        data.pixel_kind(),
                    ));
                }

                let bytes = data.data();

                const RGBA8_SIZE: usize = 16 * 16 * 16 * 4;
                const RGB8_SIZE: usize = 16 * 16 * 16 * 3;

                if data.pixel_kind() == TexturePixelKind::RGBA8 {
                    if bytes.len() != RGBA8_SIZE {
                        return Err(ColorGradingLutCreationError::NotEnoughData {
                            required: RGBA8_SIZE,
                            current: bytes.len(),
                        });
                    }
                } else if bytes.len() != RGB8_SIZE {
                    return Err(ColorGradingLutCreationError::NotEnoughData {
                        required: RGB8_SIZE,
                        current: bytes.len(),
                    });
                }

                let pixel_size = if data.pixel_kind() == TexturePixelKind::RGBA8 {
                    4
                } else {
                    3
                };

                let mut lut_bytes = Vec::with_capacity(16 * 16 * 16 * 3);

                for z in 0..16 {
                    for y in 0..16 {
                        for x in 0..16 {
                            let pixel_index = z * 16 + y * 16 * 16 + x;
                            let pixel_byte_pos = pixel_index * pixel_size;

                            lut_bytes.push(bytes[pixel_byte_pos]); // R
                            lut_bytes.push(bytes[pixel_byte_pos + 1]); // G
                            lut_bytes.push(bytes[pixel_byte_pos + 2]); // B
                        }
                    }
                }

                let lut = Texture::from_bytes(
                    TextureKind::Volume {
                        width: 16,
                        height: 16,
                        depth: 16,
                    },
                    TexturePixelKind::RGB8,
                    lut_bytes,
                    false,
                )
                .unwrap();

                let mut lut_ref = lut.data_ref();

                lut_ref.set_s_wrap_mode(TextureWrapMode::ClampToEdge);
                lut_ref.set_t_wrap_mode(TextureWrapMode::ClampToEdge);

                drop(lut_ref);
                drop(data);

                Ok(Self {
                    lut: Some(lut),
                    unwrapped_lut: Some(unwrapped_lut),
                })
            }
            Err(e) => Err(ColorGradingLutCreationError::Texture(e)),
        }
    }

    /// Returns color grading unwrapped look-up table. This is initial texture that was
    /// used to create the look-up table.
    pub fn unwrapped_lut(&self) -> Texture {
        self.unwrapped_lut.clone().unwrap()
    }

    /// Returns 3D color grading look-up table ready for use on GPU.
    pub fn lut(&self) -> Texture {
        self.lut.clone().unwrap()
    }

    /// Returns 3D color grading look-up table by ref ready for use on GPU.
    pub fn lut_ref(&self) -> &Texture {
        self.lut.as_ref().unwrap()
    }
}

/// Camera builder is used to create new camera in declarative manner.
/// This is typical implementation of Builder pattern.
pub struct CameraBuilder {
    base_builder: BaseBuilder,
    fov: f32,
    z_near: f32,
    z_far: f32,
    viewport: Rect<f32>,
    enabled: bool,
    skybox: Option<SkyBox>,
    environment: Option<Texture>,
    exposure: Exposure,
    color_grading_lut: Option<ColorGradingLut>,
    color_grading_enabled: bool,
    projection: Projection,
}

impl CameraBuilder {
    /// Creates new camera builder using given base node builder.
    pub fn new(base_builder: BaseBuilder) -> Self {
        Self {
            enabled: true,
            base_builder,
            fov: 75.0f32.to_radians(),
            z_near: 0.025,
            z_far: 2048.0,
            viewport: Rect::new(0.0, 0.0, 1.0, 1.0),
            skybox: None,
            environment: None,
            exposure: Exposure::Manual(std::f32::consts::E),
            color_grading_lut: None,
            color_grading_enabled: false,
            projection: Projection::default(),
        }
    }

    /// Sets desired field of view in radians.
    pub fn with_fov(mut self, fov: f32) -> Self {
        self.fov = fov;
        self
    }

    /// Sets desired near projection plane.
    pub fn with_z_near(mut self, z_near: f32) -> Self {
        self.z_near = z_near;
        self
    }

    /// Sets desired far projection plane.
    pub fn with_z_far(mut self, z_far: f32) -> Self {
        self.z_far = z_far;
        self
    }

    /// Sets desired viewport.
    pub fn with_viewport(mut self, viewport: Rect<f32>) -> Self {
        self.viewport = viewport;
        self
    }

    /// Sets desired initial state of camera: enabled or disabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets desired skybox.
    pub fn with_skybox(mut self, skybox: SkyBox) -> Self {
        self.skybox = Some(skybox);
        self
    }

    /// Sets desired environment map.
    pub fn with_environment(mut self, environment: Texture) -> Self {
        self.environment = Some(environment);
        self
    }

    /// Sets desired color grading LUT.
    pub fn with_color_grading_lut(mut self, lut: ColorGradingLut) -> Self {
        self.color_grading_lut = Some(lut);
        self
    }

    /// Sets whether color grading should be enabled or not.
    pub fn with_color_grading_enabled(mut self, enabled: bool) -> Self {
        self.color_grading_enabled = enabled;
        self
    }

    /// Sets desired exposure options.
    pub fn with_exposure(mut self, exposure: Exposure) -> Self {
        self.exposure = exposure;
        self
    }

    /// Sets desired projection mode.
    pub fn with_projection(mut self, projection: Projection) -> Self {
        self.projection = projection;
        self
    }

    /// Creates new instance of camera.
    pub fn build_camera(self) -> Camera {
        Camera {
            enabled: self.enabled.into(),
            base: self.base_builder.build_base(),
            projection: self.projection.into(),
            viewport: self.viewport.into(),
            // No need to calculate these matrices - they'll be automatically
            // recalculated before rendering.
            view_matrix: Matrix4::identity(),
            projection_matrix: Matrix4::identity(),
            visibility_cache: Default::default(),
            sky_box: self.skybox.into(),
            environment: self.environment.into(),
            exposure: self.exposure.into(),
            color_grading_lut: self.color_grading_lut.into(),
            color_grading_enabled: self.color_grading_enabled.into(),
        }
    }

    /// Creates new instance of camera node.
    pub fn build_node(self) -> Node {
        Node::new(self.build_camera())
    }

    /// Creates new instance of camera node and adds it to the graph.
    pub fn build(self, graph: &mut Graph) -> Handle<Node> {
        graph.add_node(self.build_node())
    }
}

/// SkyBox builder is used to create new skybox in declarative manner.
pub struct SkyBoxBuilder {
    /// Texture for front face.
    pub front: Option<Texture>,
    /// Texture for back face.
    pub back: Option<Texture>,
    /// Texture for left face.
    pub left: Option<Texture>,
    /// Texture for right face.
    pub right: Option<Texture>,
    /// Texture for top face.
    pub top: Option<Texture>,
    /// Texture for bottom face.
    pub bottom: Option<Texture>,
}

impl SkyBoxBuilder {
    /// Sets desired front face of cubemap.
    pub fn with_front(mut self, texture: Texture) -> Self {
        self.front = Some(texture);
        self
    }

    /// Sets desired back face of cubemap.
    pub fn with_back(mut self, texture: Texture) -> Self {
        self.back = Some(texture);
        self
    }

    /// Sets desired left face of cubemap.
    pub fn with_left(mut self, texture: Texture) -> Self {
        self.left = Some(texture);
        self
    }

    /// Sets desired right face of cubemap.
    pub fn with_right(mut self, texture: Texture) -> Self {
        self.right = Some(texture);
        self
    }

    /// Sets desired top face of cubemap.
    pub fn with_top(mut self, texture: Texture) -> Self {
        self.top = Some(texture);
        self
    }

    /// Sets desired front face of cubemap.
    pub fn with_bottom(mut self, texture: Texture) -> Self {
        self.bottom = Some(texture);
        self
    }

    /// Creates a new instance of skybox.
    pub fn build(self) -> Result<SkyBox, SkyBoxError> {
        let mut skybox = SkyBox {
            left: self.left,
            right: self.right,
            top: self.top,
            bottom: self.bottom,
            front: self.front,
            back: self.back,
            cubemap: None,
        };

        skybox.create_cubemap()?;

        Ok(skybox)
    }
}

/// Skybox is a huge box around camera. Each face has its own texture, when textures are
/// properly made, there is no seams and you get good decoration which contains static
/// skies and/or some other objects (mountains, buildings, etc.). Usually skyboxes used
/// in outdoor scenes, however real use of it limited only by your imagination. Skybox
/// will be drawn first, none of objects could be drawn before skybox.
#[derive(Debug, Clone, Default, PartialEq, Inspect, Reflect, Visit, Eq)]
pub struct SkyBox {
    /// Texture for front face.
    #[reflect(setter = "set_front")]
    pub(crate) front: Option<Texture>,

    /// Texture for back face.
    #[reflect(setter = "set_back")]
    pub(crate) back: Option<Texture>,

    /// Texture for left face.
    #[reflect(setter = "set_left")]
    pub(crate) left: Option<Texture>,

    /// Texture for right face.
    #[reflect(setter = "set_right")]
    pub(crate) right: Option<Texture>,

    /// Texture for top face.
    #[reflect(setter = "set_top")]
    pub(crate) top: Option<Texture>,

    /// Texture for bottom face.
    #[reflect(setter = "set_bottom")]
    pub(crate) bottom: Option<Texture>,

    /// Cubemap texture
    #[inspect(skip)]
    #[visit(skip)]
    #[reflect(hidden)]
    pub(crate) cubemap: Option<Texture>,
}

/// An error that may occur during skybox creation.
#[derive(Debug)]
pub enum SkyBoxError {
    /// Texture kind is not TextureKind::Rectangle
    UnsupportedTextureKind(TextureKind),
    /// Cube map was failed to build.
    UnableToBuildCubeMap,
    /// Input texture is not square.
    NonSquareTexture {
        /// Texture index.
        index: usize,
        /// Width of the faulty texture.
        width: u32,
        /// Height of the faulty texture.
        height: u32,
    },
    /// Some input texture differs in size or pixel kind.
    DifferentTexture {
        /// Actual width of the first valid texture in the input set.
        expected_width: u32,
        /// Actual height of the first valid texture in the input set.
        expected_height: u32,
        /// Actual pixel kind of the first valid texture in the input set.
        expected_pixel_kind: TexturePixelKind,
        /// Index of the faulty input texture.
        index: usize,
        /// Width of the faulty texture.
        actual_width: u32,
        /// Height of the faulty texture.
        actual_height: u32,
        /// Pixel kind of the faulty texture.
        actual_pixel_kind: TexturePixelKind,
    },
    /// Occurs when one of the input textures is either still loading or failed to load.
    TextureIsNotReady {
        /// Index of the faulty input texture.
        index: usize,
    },
}

impl SkyBox {
    /// Returns cubemap texture
    pub fn cubemap(&self) -> Option<Texture> {
        self.cubemap.clone()
    }

    /// Returns cubemap texture
    pub fn cubemap_ref(&self) -> Option<&Texture> {
        self.cubemap.as_ref()
    }

    /// Validates input set of texture and checks if it possible to create a cube map from them.
    /// There are two main conditions for successful cube map creation:
    /// - All textures must have same width and height, and width must be equal to height.
    /// - All textures must have same pixel kind.
    pub fn validate(&self) -> Result<(), SkyBoxError> {
        struct TextureInfo {
            pixel_kind: TexturePixelKind,
            width: u32,
            height: u32,
        }

        let mut first_info: Option<TextureInfo> = None;

        for (index, texture) in self.textures().iter().enumerate() {
            if let Some(texture) = texture {
                if let ResourceState::Ok(texture) = &*texture.state() {
                    if let TextureKind::Rectangle { width, height } = texture.kind() {
                        if width != height {
                            return Err(SkyBoxError::NonSquareTexture {
                                index,
                                width,
                                height,
                            });
                        }

                        if let Some(first_info) = first_info.as_mut() {
                            if first_info.width != width
                                || first_info.height != height
                                || first_info.pixel_kind != texture.pixel_kind()
                            {
                                return Err(SkyBoxError::DifferentTexture {
                                    expected_width: first_info.width,
                                    expected_height: first_info.height,
                                    expected_pixel_kind: first_info.pixel_kind,
                                    index,
                                    actual_width: width,
                                    actual_height: height,
                                    actual_pixel_kind: texture.pixel_kind(),
                                });
                            }
                        } else {
                            first_info = Some(TextureInfo {
                                pixel_kind: texture.pixel_kind(),
                                width,
                                height,
                            });
                        }
                    }
                } else {
                    return Err(SkyBoxError::TextureIsNotReady { index });
                }
            }
        }

        Ok(())
    }

    /// Creates a cubemap using provided faces. If some face has not been provided corresponding side will be black.
    ///
    /// # Important notes.
    ///
    /// It will fail if provided face's kind is not TextureKind::Rectangle.
    pub fn create_cubemap(&mut self) -> Result<(), SkyBoxError> {
        self.validate()?;

        let (kind, pixel_kind, bytes_per_face) =
            self.textures().iter().find(|face| face.is_some()).map_or(
                (
                    TextureKind::Rectangle {
                        width: 1,
                        height: 1,
                    },
                    TexturePixelKind::R8,
                    1,
                ),
                |face| {
                    let face = face.clone().unwrap();
                    let data = face.data_ref();

                    (
                        data.kind(),
                        data.pixel_kind(),
                        data.first_mip_level_data().len(),
                    )
                },
            );

        let (width, height) = match kind {
            TextureKind::Rectangle { width, height } => (width, height),
            _ => return Err(SkyBoxError::UnsupportedTextureKind(kind)),
        };

        let mut data = Vec::<u8>::with_capacity(bytes_per_face * 6);
        for face in self.textures().iter() {
            if let Some(f) = face.clone() {
                data.extend(f.data_ref().first_mip_level_data());
            } else {
                let black_face_data = vec![0; bytes_per_face];
                data.extend(black_face_data);
            }
        }

        self.cubemap = Some(
            Texture::from_bytes(TextureKind::Cube { width, height }, pixel_kind, data, false)
                .ok_or(SkyBoxError::UnableToBuildCubeMap)?,
        );

        Ok(())
    }

    /// Returns slice with all textures, where: 0 - Left, 1 - Right, 2 - Top, 3 - Bottom
    /// 4 - Front, 5 - Back.
    ///
    /// # Important notes.
    ///
    /// These textures are **not** used for rendering! The renderer uses cube map made of these
    /// textures. Public access for these textures is needed in case you need to read internals
    /// of the textures.
    pub fn textures(&self) -> [Option<Texture>; 6] {
        [
            self.left.clone(),
            self.right.clone(),
            self.top.clone(),
            self.bottom.clone(),
            self.front.clone(),
            self.back.clone(),
        ]
    }

    /// Set new texture for the left side of the skybox.
    pub fn set_left(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.left, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for left face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn left(&self) -> Option<Texture> {
        self.left.clone()
    }

    /// Set new texture for the right side of the skybox.
    pub fn set_right(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.right, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for right face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn right(&self) -> Option<Texture> {
        self.right.clone()
    }

    /// Set new texture for the top side of the skybox.
    pub fn set_top(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.top, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for top face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn top(&self) -> Option<Texture> {
        self.top.clone()
    }

    /// Set new texture for the bottom side of the skybox.
    pub fn set_bottom(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.bottom, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for bottom face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn bottom(&self) -> Option<Texture> {
        self.bottom.clone()
    }

    /// Set new texture for the front side of the skybox.
    pub fn set_front(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.front, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for front face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn front(&self) -> Option<Texture> {
        self.front.clone()
    }

    /// Set new texture for the back side of the skybox.
    pub fn set_back(&mut self, texture: Option<Texture>) -> Option<Texture> {
        let prev = std::mem::replace(&mut self.back, texture);
        Log::verify(self.create_cubemap());
        prev
    }

    /// Returns a texture that is used for back face of the cube map.
    ///
    /// # Important notes.
    ///
    /// This textures is not used for rendering! The renderer uses cube map made of face textures.
    pub fn back(&self) -> Option<Texture> {
        self.back.clone()
    }
}
