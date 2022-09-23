#[derive(Debug, Clone)]
pub struct KalmanFilter {
    state: Option<Gaussian>,
    state_sigma: f32,
    noise_sigma: f32,
}

impl KalmanFilter {
    pub fn new(state_sigma: f32, noise_sigma: f32) -> KalmanFilter {
        KalmanFilter {
            state: None,
            state_sigma,
            noise_sigma,
        }
    }

    pub fn predict(&mut self, observation: f32) -> f32 {
        if let Some(ref mut state) = self.state {
            let prior = Gaussian::new(state.mu, state.sigma + self.noise_sigma);
            let gain = prior.sigma / (prior.sigma + self.state_sigma);
            *state = Gaussian::new(
                prior.mu + gain * (observation - prior.mu),
                (1.0 - gain) * prior.sigma,
            );
            return state.mu;
        }

        self.state = Some(Gaussian::new(observation, self.state_sigma));
        observation
    }
}

#[derive(Debug, Clone, Copy)]
struct Gaussian {
    mu: f32,
    sigma: f32,
}

impl Gaussian {
    fn new(mu: f32, sigma: f32) -> Self {
        Self { mu, sigma }
    }
}
