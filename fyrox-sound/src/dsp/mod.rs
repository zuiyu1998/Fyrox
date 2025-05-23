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

// Clippy is being stupid here again, filters cannot be empty and there is no
// need to define is_empty() method.
#![allow(clippy::len_without_is_empty)]

//! Digital signal processing module. Provides basic elements to process signal sample-by-sample.
//!
//! # Abbreviations
//!
//! `fc` - normalized frequency, i.e. `fc = 0.2` with `sample rate = 44100 Hz` will be `f = 8820 Hz`

use fyrox_core::visitor::pod::PodVecView;
use fyrox_core::visitor::{Visit, VisitResult, Visitor};

pub mod filters;

#[derive(Debug, PartialEq, Clone)]
struct SamplesContainer(pub Vec<f32>);

impl Visit for SamplesContainer {
    fn visit(&mut self, name: &str, visitor: &mut Visitor) -> VisitResult {
        PodVecView::from_pod_vec(&mut self.0).visit(name, visitor)
    }
}

/// See more info here <https://ccrma.stanford.edu/~jos/pasp/Delay_Lines.html>
#[derive(Debug, PartialEq, Clone, Visit)]
pub struct DelayLine {
    #[visit(optional)]
    samples: SamplesContainer,
    last: f32,
    pos: u32,
}

impl DelayLine {
    /// Creates new instance of delay line of given length in samples.
    pub fn new(len: usize) -> Self {
        Self {
            samples: SamplesContainer(vec![0.0; len]),
            last: 0.0,
            pos: 0,
        }
    }

    /// Returns length of delay line in samples.
    pub fn len(&self) -> usize {
        self.samples.0.len()
    }

    /// Processes single sample.
    pub fn feed(&mut self, sample: f32) -> f32 {
        self.last = self.samples.0[self.pos as usize];
        self.samples.0[self.pos as usize] = sample;
        self.pos += 1;
        if self.pos >= self.samples.0.len() as u32 {
            self.pos -= self.samples.0.len() as u32
        }
        self.last
    }

    /// Returns last processed sample.
    pub fn last(&self) -> f32 {
        self.last
    }
}

impl Default for DelayLine {
    fn default() -> Self {
        Self {
            samples: SamplesContainer(vec![0.0]),
            last: 0.0,
            pos: 0,
        }
    }
}

/// Calculates single coefficient of Hamming window.
/// <https://en.wikipedia.org/wiki/Window_function#Hamming_window>
pub fn hamming_window(i: usize, sample_count: usize) -> f32 {
    0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (sample_count - 1) as f32).cos()
}

/// Calculates single coefficient of Hann window.
/// <https://en.wikipedia.org/wiki/Hann_function>
pub fn hann_window(i: usize, sample_count: usize) -> f32 {
    0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (sample_count - 1) as f32).cos()
}

/// Creates new window using specified window function.
/// <https://en.wikipedia.org/wiki/Window_function>
pub fn make_window<W: Fn(usize, usize) -> f32>(sample_count: usize, func: W) -> Vec<f32> {
    (0..sample_count).map(|i| func(i, sample_count)).collect()
}
